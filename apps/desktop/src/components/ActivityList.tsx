import { CheckCheck, Copy, File, ArrowDownLeft, ArrowUpRight } from 'lucide-react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { Activity, Peer } from '../types';
import { useEffect, useRef } from 'react';

export function formatSize(bytes: number): string {
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

const statuses: Record<string, string> = {
  connecting: '正在连接',
  awaiting_accept: '等待接收确认',
  transferring: '传输中',
  verifying: '正在校验并保存',
  completed: '已完成',
  failed: '未完成',
  rejected: '已拒绝',
};
interface Props {
  activities: Activity[];
  peers: Peer[];
  onDecide: (id: string, accepted: boolean) => void;
  onError: (message: string) => void;
}
export function ActivityList({ activities, peers, onDecide, onError }: Props) {
  const end = useRef<HTMLDivElement>(null);
  const latest = activities.at(-1)?.id;
  useEffect(() => {
    end.current?.scrollIntoView({ block: 'end' });
  }, [latest]);
  return (
    <div className="activity-list">
      {activities.map((activity) => {
        const incoming = activity.direction === 'incoming';
        const peer = peers.find((peer) => peer.id === activity.peerId);
        const progress = activity.size
          ? Math.min(100, (activity.transferred / activity.size) * 100)
          : activity.status === 'completed'
            ? 100
            : 0;
        return (
          <article
            className={`activity ${incoming ? 'incoming' : 'outgoing'}`}
            key={`${activity.direction}:${activity.peerId}:${activity.id}`}
          >
            <div className="activity-meta">
              <span>
                {incoming ? <ArrowDownLeft size={13} /> : <ArrowUpRight size={13} />}
                {incoming ? '来自' : '发送至'} {peer?.name ?? '已移除设备'}
              </span>
              <time>
                {new Date(activity.timestamp).toLocaleTimeString('zh-CN', {
                  hour: '2-digit',
                  minute: '2-digit',
                })}
              </time>
            </div>
            {activity.kind === 'text' ? (
              <div className="text-bubble">
                <p>{activity.content}</p>
                <button
                  className="icon-button"
                  aria-label="复制文字"
                  title="复制文字"
                  onClick={() => {
                    void writeText(activity.content).catch((error) => onError(String(error)));
                  }}
                >
                  <Copy size={15} />
                </button>
              </div>
            ) : (
              <div className="file-card">
                <div className="file-icon">
                  <File size={23} />
                </div>
                <div className="file-details">
                  <strong>{activity.content}</strong>
                  <span>
                    {formatSize(activity.size)} · {statuses[activity.status] ?? activity.status}
                  </span>
                  {!['completed', 'failed', 'rejected', 'awaiting_accept'].includes(
                    activity.status,
                  ) && (
                    <>
                      <progress max={100} value={progress} />
                      <span>
                        {formatSize(activity.transferred)} / {formatSize(activity.size)}
                      </span>
                    </>
                  )}
                  {incoming && activity.status === 'completed' && (
                    <small className="saved-path" title={activity.path ?? ''}>
                      {activity.path}
                    </small>
                  )}
                </div>
                {activity.status === 'completed' && <CheckCheck className="success" size={19} />}
              </div>
            )}
            {incoming && activity.kind === 'file' && activity.status === 'awaiting_accept' && (
              <div className="receipt-actions">
                <button
                  className="button secondary small"
                  onClick={() => onDecide(activity.id, false)}
                >
                  拒绝
                </button>
                <button
                  className="button primary small"
                  onClick={() => onDecide(activity.id, true)}
                >
                  接收文件
                </button>
              </div>
            )}
            {activity.kind === 'text' && (
              <small className={`delivery ${activity.status === 'failed' ? 'failure' : ''}`}>
                {statuses[activity.status] ?? activity.status}
              </small>
            )}
            {activity.error && <p className="inline-error">{activity.error}</p>}
          </article>
        );
      })}
      <div ref={end} />
    </div>
  );
}
