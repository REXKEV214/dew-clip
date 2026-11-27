import Foundation

public typealias DownloadProgress = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int
) -> Bool

private struct DownloadPolicy: Decodable {
    let base: URL
    let hosts: [String]
    let size: Int
    let idle: Double
    let grace: Double
    let sample: Double
    let window: Double
    let minimumRate: Double
    let total: Double

    func permits(_ url: URL, initial: Bool) -> Bool {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let host = parts.host, hosts.contains(host),
              parts.scheme == base.scheme, parts.user == nil, parts.password == nil,
              parts.port == base.port || (!initial && base.scheme == "https" && parts.port == 443)
        else { return false }
        if initial && host != base.host { return false }
        if host != base.host { return true }
        let path = parts.path
        let basePath = base.path.hasSuffix("/") ? base.path : base.path + "/"
        return path.hasPrefix(basePath) && !path.contains("\\")
            && !path.split(separator: "/").contains(where: { $0 == "." || $0 == ".." })
    }
}

private func downloadRequest(_ url: URL, timeout: Double) -> URLRequest {
    var request = URLRequest(url: url, timeoutInterval: timeout)
    request.setValue("DewClip", forHTTPHeaderField: "User-Agent")
    request.setValue("", forHTTPHeaderField: "Accept-Language")
    return request
}

private enum DownloadResult: Int32 {
    case failed = -1, slow = -2, restricted = -3, storage = -4, cancelled = -5
}

// All delegate methods and the timer run on the same serial queue.
private final class AssetDownload: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    let completed = DispatchSemaphore(value: 0)
    var result = DownloadResult.failed.rawValue
    private let policy: DownloadPolicy
    private let file: FileHandle
    private let context: UnsafeMutableRawPointer?
    private let progress: DownloadProgress
    private let started = ProcessInfo.processInfo.systemUptime
    private var status: Int32 = 0
    private var failure: DownloadResult?
    private var redirects = 0
    private var received = 0
    private var lastActivity: Double
    private var bodyStartedAt: Double?
    private var nextSample: Double?
    private var slowSamples = 0
    private var rateHistory: [(time: Double, bytes: Int)] = []
    private var finished = false

    init(policy: DownloadPolicy, fd: Int32, context: UnsafeMutableRawPointer?,
         progress: @escaping DownloadProgress) {
        self.policy = policy
        self.file = FileHandle(fileDescriptor: fd, closeOnDealloc: false)
        self.context = context
        self.progress = progress
        self.lastActivity = started
    }

    private func stop(_ task: URLSessionTask, _ reason: DownloadResult) {
        failure = reason
        task.cancel()
    }

    private func recordBodyActivity(at now: Double) {
        lastActivity = now
        if bodyStartedAt == nil {
            bodyStartedAt = now
            nextSample = now + policy.grace
        }
    }

    func tick(_ task: URLSessionTask) {
        guard !finished, failure == nil else { return }
        let now = ProcessInfo.processInfo.systemUptime
        if !progress(context, nil, 0) {
            stop(task, .cancelled)
        } else if now - started >= policy.total || now - lastActivity >= policy.idle {
            stop(task, .slow)
        } else if let bodyStartedAt, let nextSample, now >= nextSample {
            let cutoff = now - policy.window
            rateHistory.removeAll { $0.time < cutoff }
            let bytes = rateHistory.reduce(0) { $0 + $1.bytes }
            let rate = Double(bytes) / min(policy.window, now - bodyStartedAt)
            slowSamples = rate < policy.minimumRate ? slowSamples + 1 : 0
            self.nextSample = now + policy.sample
            if slowSamples >= 2 { stop(task, .slow) }
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        guard failure == nil, redirects < 3, let url = request.url, policy.permits(url, initial: false) else {
            if failure == nil { stop(task, .failed) }
            completionHandler(nil)
            return
        }
        redirects += 1
        completionHandler(downloadRequest(url, timeout: policy.idle))
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard failure == nil, let response = response as? HTTPURLResponse else {
            completionHandler(.cancel)
            return
        }
        status = Int32(response.statusCode)
        guard status == 200, response.expectedContentLength <= Int64(policy.size) else {
            if status == 200 { failure = .failed }
            completionHandler(.cancel)
            return
        }
        recordBodyActivity(at: ProcessInfo.processInfo.systemUptime)
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard failure == nil, !data.isEmpty else { return }
        guard data.count <= policy.size - received else { stop(dataTask, .failed); return }
        let now = ProcessInfo.processInfo.systemUptime
        recordBodyActivity(at: now)
        do {
            try file.write(contentsOf: data)
        } catch {
            stop(dataTask, .storage)
            return
        }
        let keepDownloading = data.withUnsafeBytes { bytes in
            progress(context, bytes.bindMemory(to: UInt8.self).baseAddress, data.count)
        }
        guard keepDownloading else { stop(dataTask, .cancelled); return }
        received += data.count
        rateHistory.append((now, data.count))
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        finished = true
        if let failure {
            result = failure.rawValue
        } else if status != 0 && status != 200 {
            result = status
        } else if let error = error as? URLError {
            switch error.networkUnavailableReason {
            case .constrained, .expensive: result = DownloadResult.restricted.rawValue
            default:
                if error.code == .timedOut { result = DownloadResult.slow.rawValue }
            }
        } else if error == nil && status == 200 && received == policy.size {
            result = status
        }
        completed.signal()
    }
}

// Rust owns the private cache file. Writing to its descriptor avoids reopening the path.
@_cdecl("dewclip_download_update")
public func downloadUpdate(_ url: UnsafePointer<CChar>, _ options: UnsafePointer<CChar>, _ fd: Int32,
                           _ manual: Bool, _ context: UnsafeMutableRawPointer?,
                           _ progress: @escaping DownloadProgress) -> Int32 {
    guard let url = URL(string: String(cString: url)),
          let policy = try? JSONDecoder().decode(DownloadPolicy.self, from: Data(String(cString: options).utf8)),
          policy.size > 0, policy.size <= 64 * 1024 * 1024, policy.permits(url, initial: true)
    else { return DownloadResult.failed.rawValue }

    let configuration = URLSessionConfiguration.ephemeral
    configuration.timeoutIntervalForRequest = policy.idle
    configuration.timeoutIntervalForResource = policy.total
    configuration.urlCache = nil
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.urlCredentialStorage = nil
    configuration.waitsForConnectivity = false
    configuration.allowsExpensiveNetworkAccess = manual
    configuration.allowsConstrainedNetworkAccess = manual

    let queue = DispatchQueue(label: "dewclip.update-download")
    let operations = OperationQueue()
    operations.maxConcurrentOperationCount = 1
    operations.underlyingQueue = queue
    let delegate = AssetDownload(policy: policy, fd: fd, context: context, progress: progress)
    let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: operations)
    let task = session.dataTask(with: downloadRequest(url, timeout: policy.idle))
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now(), repeating: min(0.1, policy.idle / 4, policy.sample / 4))
    timer.setEventHandler { delegate.tick(task) }
    timer.resume()
    task.resume()
    delegate.completed.wait()
    timer.cancel()
    session.finishTasksAndInvalidate()
    return delegate.result
}
