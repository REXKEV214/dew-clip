import { useEffect, useRef, useState } from "react";
import { Reorder, useDragControls } from "framer-motion";
import { FileText, Film, GripVertical, Image, X } from "lucide-react";
import { useImageThumbnail, useVideoThumbnail } from "../../shared/hooks/useImageThumbnail";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import type { Translate } from "../../shared/types";
import type { SequentialItem } from "./useSequentialQueue";

interface QueuePanelProps {
  t: Translate;
  items: SequentialItem[];
  hotkey: string;
  onReorder: (queueIds: number[]) => void;
  onRemove: (queueId: number) => void;
  onClear: () => void;
  onEnd: () => void;
  onPaste: () => void;
}

function QueuePreview({ item, t }: { item: SequentialItem; t: Translate }) {
  const image = useImageThumbnail(item.contentType === "image" ? item.mediaPath : null, true, item.timestamp);
  const video = useVideoThumbnail(item.contentType === "video" ? item.mediaPath : null, item.timestamp);
  const thumb = image || video;
  return (
    <span className="batch-content queue-preview">
      {thumb ? <img className="batch-image" src={thumb} alt="" draggable={false} /> :
        item.contentType === "image" ? <Image size={15} /> :
        item.contentType === "video" ? <Film size={15} /> :
        item.contentType === "file" ? <FileText size={15} /> : null}
      <span className="batch-text">{item.preview || t(`type_${item.contentType}`)}</span>
    </span>
  );
}

function QueueRow({ item, index, t, onRemove, onDragEnd, onMove }: {
  item: SequentialItem;
  index: number;
  t: Translate;
  onRemove: () => void;
  onDragEnd: () => void;
  onMove: (delta: number) => void;
}) {
  const dragControls = useDragControls();
  return (
    <Reorder.Item value={item} dragListener={false} dragControls={dragControls} onDragEnd={onDragEnd} className="batch-row">
      <div className={`batch-item queue-item${index === 0 ? " is-next" : ""}`}>
        <button
          type="button"
          className="card-action favorite-grip"
          data-queue-reorder
          title={t("favorites_drag")}
          aria-label={t("favorites_drag")}
          onPointerDown={(event) => dragControls.start(event)}
          onKeyDown={(event) => {
            if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
            event.preventDefault();
            event.stopPropagation();
            onMove(event.key === "ArrowUp" ? -1 : 1);
          }}
        >
          <GripVertical size={14} />
        </button>
        <span className="queue-position">{index + 1}</span>
        <QueuePreview item={item} t={t} />
        {index === 0 && <span className="queue-next">{t("sequential_next_badge")}</span>}
        <button type="button" className="card-action" title={t("batch_remove")} aria-label={t("batch_remove")} onClick={onRemove}>
          <X size={12} />
        </button>
      </div>
    </Reorder.Item>
  );
}

export default function QueuePanel({ t, items, hotkey, onReorder, onRemove, onClear, onEnd, onPaste }: QueuePanelProps) {
  const [draft, setDraft] = useState<SequentialItem[] | null>(null);
  const pendingOrder = useRef<SequentialItem[] | null>(null);
  useEffect(() => {
    pendingOrder.current = null;
    setDraft(null);
  }, [items]);
  const ordered = draft ?? items;

  const stageOrder = (next: SequentialItem[]) => {
    pendingOrder.current = next;
    setDraft(next);
  };

  const finishDrag = () => {
    const next = pendingOrder.current;
    pendingOrder.current = null;
    setDraft(null);
    if (next) onReorder(next.map((item) => item.queueId));
  };

  const moveItem = (index: number, offset: number) => {
    const destination = index + offset;
    if (destination < 0 || destination >= ordered.length) return;
    const next = ordered.slice();
    [next[index], next[destination]] = [next[destination], next[index]];
    onReorder(next.map((item) => item.queueId));
  };

  const status = items.length
    ? t(hotkey ? "sequential_status_ready" : "sequential_status_count")
      .replace("{count}", String(items.length)).replace("{hotkey}", hotkey)
    : t("sequential_empty_hint");
  const pasteLabel = [t("sequential_paste_next"), hotkey && `(${hotkey})`].filter(Boolean).join(" ");

  return (
    <div className="side-panel-body">
      <div className="panel-title" onMouseDown={startWindowDrag}>
        <h2>{t("sequential_title")}</h2>
        <button type="button" className="glass-button is-compact" disabled={!items.length} onClick={onClear}>
          {t("sequential_clear")}
        </button>
      </div>
      <p className="queue-status" title={!hotkey ? t("sequential_hotkey_empty") : undefined}>{status}</p>
      <div className="panel-fill">
        <Reorder.Group axis="y" values={ordered} onReorder={stageOrder} className="batch-list batch-preview queue-scroll">
          {ordered.map((item, index) => (
            <QueueRow
              key={item.queueId}
              item={item}
              index={index}
              t={t}
              onRemove={() => onRemove(item.queueId)}
              onDragEnd={finishDrag}
              onMove={(delta) => moveItem(index, delta)}
            />
          ))}
        </Reorder.Group>
      </div>
      <div className="panel-actions">
        <button type="button" className="glass-button" onClick={onEnd}>{t("sequential_stop")}</button>
        <button type="button" className="glass-button is-primary" disabled={!items.length} onClick={onPaste}>{pasteLabel}</button>
      </div>
    </div>
  );
}
