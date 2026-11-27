use chrono::{DateTime, Utc};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
static LOG_SINK: Mutex<Option<LogSink>> = Mutex::new(None);

struct LogSink {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
}

impl LogSink {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        let mut sink = Self {
            path,
            file: None,
            bytes: 0,
            max_bytes,
        };
        sink.open_current()?;
        if sink.bytes > max_bytes {
            sink.rotate()?;
        }
        Ok(sink)
    }

    fn open_current(&mut self) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.bytes = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.flush()?;
        let mut backup = self.path.as_os_str().to_os_string();
        backup.push(".1");
        match fs::rename(&self.path, backup) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.file = None;
        self.open_current()
    }

    fn write_line(&mut self, mut line: String) -> io::Result<()> {
        let mut end = (line.len() as u64).min(self.max_bytes - 1) as usize;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
        line.push('\n');
        let line_bytes = line.len() as u64;

        if self.file.is_none() {
            self.open_current()?;
        }
        if self.bytes + line_bytes > self.max_bytes {
            self.rotate()?;
        }

        let result = self.file.as_mut().unwrap().write_all(line.as_bytes());
        if result.is_ok() {
            self.bytes += line_bytes;
        } else {
            // 可能只写入了部分数据，下次打开时重新读取大小。
            self.file = None;
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

fn lock_sink(sink: &Mutex<Option<LogSink>>) -> MutexGuard<'_, Option<LogSink>> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}

fn report_error(action: &str, error: io::Error) {
    if cfg!(debug_assertions) {
        eprintln!("Could not {action} log: {error}");
    }
}

pub fn init(path: PathBuf) {
    let mut sink = lock_sink(&LOG_SINK);
    match LogSink::open(path, MAX_LOG_BYTES) {
        Ok(log) => *sink = Some(log),
        Err(error) => report_error("open", error),
    }
}

pub fn flush() {
    if let Some(sink) = lock_sink(&LOG_SINK).as_mut() {
        if let Err(error) = sink.flush() {
            report_error("flush", error);
        }
    }
}

pub fn log(msg: &str) {
    if let Some(sink) = lock_sink(&LOG_SINK).as_mut() {
        let now: DateTime<Utc> = SystemTime::now().into();
        let timestamp = now.format("%Y-%m-%d %H:%M:%S%.3f");
        if let Err(error) = sink.write_line(format!("[{timestamp}] {msg}")) {
            report_error("write", error);
        }
    }
    #[cfg(debug_assertions)]
    println!("{msg}");
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::logger::log(&format!($($arg)*))
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::logger::log(&format!("[ERROR] {}", format!($($arg)*)))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_log_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dewclip-log-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn rotates_before_a_write_exceeds_the_cap_and_replaces_the_backup() {
        let dir = temporary_log_dir();
        let path = dir.join("dewclip.log");
        let backup = dir.join("dewclip.log.1");
        let mut sink = LogSink::open(path.clone(), 16).unwrap();

        sink.write_line("old log".into()).unwrap();
        sink.write_line("old end".into()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 16);
        assert!(!backup.exists());

        sink.write_line("middle".into()).unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old log\nold end\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "middle\n");

        sink.write_line("complete".into()).unwrap();
        sink.write_line("newest".into()).unwrap();
        sink.write_line("last".into()).unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "middle\ncomplete\n");
        assert_eq!(fs::read_to_string(path).unwrap(), "newest\nlast\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);

        drop(sink);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rotates_an_oversized_log_at_startup() {
        let dir = temporary_log_dir();
        let path = dir.join("dewclip.log");
        let backup = dir.join("dewclip.log.1");
        fs::write(&path, "log from a previous session\n").unwrap();
        fs::write(&backup, "older backup\n").unwrap();

        let mut sink = LogSink::open(path.clone(), 16).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        sink.write_line("new session".into()).unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "new session\n");
        assert_eq!(
            fs::read_to_string(backup).unwrap(),
            "log from a previous session\n"
        );

        drop(sink);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rotation_reopens_a_log_deleted_while_running() {
        let dir = temporary_log_dir();
        let path = dir.join("dewclip.log");
        let mut sink = LogSink::open(path.clone(), 16).unwrap();
        sink.write_line("old log".into()).unwrap();
        fs::remove_file(&path).unwrap();

        sink.write_line("replacement".into()).unwrap();
        sink.write_line("new".into()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "replacement\nnew\n");

        drop(sink);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn caps_an_oversized_line_at_a_utf8_boundary() {
        let dir = temporary_log_dir();
        let path = dir.join("dewclip.log");
        let mut sink = LogSink::open(path.clone(), 7).unwrap();

        sink.write_line("ab复制".into()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "ab复\n");

        drop(sink);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovers_a_poisoned_sink_lock() {
        let mutex = Mutex::new(None);
        let result = std::panic::catch_unwind(|| {
            let _sink = lock_sink(&mutex);
            panic!("poison the log lock");
        });
        assert!(result.is_err());
        assert!(mutex.is_poisoned());
        assert!(lock_sink(&mutex).is_none());
    }
}
