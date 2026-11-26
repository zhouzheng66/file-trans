import { useRef } from 'react';
import { CheckCheck, File, Folder } from 'lucide-react';
import type { Activity } from '../types';
import { hasLocalAttachment, previewKind } from '../attachments';
import { MediaPreview } from './MediaPreview';

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[index]}`;
}

interface Props {
  activity: Activity;
  statusLabel: string;
  route: string;
  onDragFile: (activity: Activity) => void;
}

export function FileCard({ activity, statusLabel, route, onDragFile }: Props) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const available = hasLocalAttachment(activity);
  const folder = activity.kind === 'folder';
  const preparingFolder = folder && activity.status === 'preparing';
  const media = available && !folder && activity.path ? previewKind(activity.path) : null;
  const progress = activity.size ? Math.min(100, (activity.transferred / activity.size) * 100) : 0;

  return (
    <div
      className={`file-card${available ? ' can-drag' : ''}`}
      title={available ? '拖到其他位置以复制' : undefined}
      onDragStart={(event) => event.preventDefault()}
      onPointerDown={(event) => {
        if (!available || !event.isPrimary || event.button !== 0 || event.pointerType === 'touch')
          return;
        // 视频的播放、音量和进度条交给系统播放器处理。
        if (event.target instanceof Element && event.target.closest('video')) return;
        start.current = { x: event.clientX, y: event.clientY };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!start.current || !event.isPrimary || !(event.buttons & 1)) return;
        if (Math.hypot(event.clientX - start.current.x, event.clientY - start.current.y) < 5)
          return;
        start.current = null;
        event.currentTarget.releasePointerCapture(event.pointerId);
        onDragFile(activity);
      }}
      onLostPointerCapture={() => {
        start.current = null;
      }}
    >
      {media && <MediaPreview key={activity.path} activity={activity} kind={media} />}
      <div className="file-summary">
        <div className="file-icon">{folder ? <Folder size={23} /> : <File size={23} />}</div>
        <div className="file-details">
          <strong>
            {activity.content}
            {folder && ' · 文件夹'}
          </strong>
          <span>
            {preparingFolder && !activity.size ? '大小统计中' : formatSize(activity.size)} ·{' '}
            {preparingFolder ? '正在准备目录快照' : statusLabel}
            {route && ` · ${route}`}
          </span>
          {!['completed', 'failed', 'rejected', 'cancelled', 'awaiting_accept'].includes(
            activity.status,
          ) && (
            <>
              <progress max={100} value={progress} />
              <span>
                {formatSize(activity.transferred)} / {formatSize(activity.size)}
              </span>
            </>
          )}
          {activity.resumedFrom > 0 && (
            <small className="saved-path">已从 {formatSize(activity.resumedFrom)} 断点继续</small>
          )}
          {activity.direction === 'incoming' && activity.status === 'completed' && (
            <small className="saved-path" title={activity.path ?? ''}>
              {activity.path}
            </small>
          )}
        </div>
        {activity.status === 'completed' && <CheckCheck className="success" size={19} />}
      </div>
    </div>
  );
}
