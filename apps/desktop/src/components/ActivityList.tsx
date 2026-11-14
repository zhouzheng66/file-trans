import { CheckCheck, Copy, File, Folder, ArrowDownLeft, ArrowUpRight } from 'lucide-react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { Activity, Peer } from '../types';
import { useEffect, useRef } from 'react';
import { TransferActions } from './TransferActions';

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
  preparing: '正在校验源文件',
  paused: '已暂停',
  paused_by_peer: '对方已暂停',
  waiting_retry: '等待网络恢复，自动续传',
  waiting_connection: '断点已保存，等待续传',
  cancelled: '已取消',
};
interface Props {
  activities: Activity[];
  peers: Peer[];
  onDecide: (activity: Activity, accepted: boolean) => void;
  onError: (message: string) => void;
  onTransferAction: (
    activity: Activity,
    action: 'pause_transfer' | 'resume_transfer' | 'cancel_transfer',
  ) => void;
}
export function ActivityList({ activities, peers, onDecide, onError, onTransferAction }: Props) {
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
        const route =
          activity.route === 'lan' ? '局域网直连' : activity.route === 'relay' ? '互联网中继' : '';
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
                  {activity.kind === 'folder' ? <Folder size={23} /> : <File size={23} />}
                </div>
                <div className="file-details">
                  <strong>
                    {activity.content}
                    {activity.kind === 'folder' && ' · 文件夹'}
                  </strong>
                  <span>
                    {activity.kind === 'folder' && activity.status === 'preparing' && !activity.size
                      ? '大小统计中'
                      : formatSize(activity.size)}{' '}
                    ·{' '}
                    {activity.kind === 'folder' && activity.status === 'preparing'
                      ? '正在准备目录快照'
                      : (statuses[activity.status] ?? activity.status)}
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
                    <small className="saved-path">
                      已从 {formatSize(activity.resumedFrom)} 断点继续
                    </small>
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
            {incoming && activity.kind !== 'text' && activity.status === 'awaiting_accept' && (
              <div className="receipt-actions">
                <button
                  className="button secondary small"
                  onClick={() => onDecide(activity, false)}
                >
                  拒绝
                </button>
                <button className="button primary small" onClick={() => onDecide(activity, true)}>
                  {activity.kind === 'folder' ? '接收文件夹' : '接收文件'}
                </button>
              </div>
            )}
            {activity.kind === 'text' && (
              <small className={`delivery ${activity.status === 'failed' ? 'failure' : ''}`}>
                {statuses[activity.status] ?? activity.status}
                {route && ` · ${route}`}
              </small>
            )}
            {activity.error && <p className="inline-error">{activity.error}</p>}
            {activity.kind !== 'text' && (
              <TransferActions activity={activity} onAction={onTransferAction} />
            )}
          </article>
        );
      })}
      <div ref={end} />
    </div>
  );
}
