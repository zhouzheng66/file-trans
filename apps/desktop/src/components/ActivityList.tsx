import { Copy, FolderOpen, ArrowDownLeft, ArrowUpRight } from 'lucide-react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { Activity, Peer } from '../types';
import { useEffect, useRef } from 'react';
import { TransferActions } from './TransferActions';
import { FileCard } from './FileCard';

const statuses: Record<string, string> = {
  connecting: '正在连接',
  awaiting_accept: '等待对端接收',
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
  onError: (message: string) => void;
  onDragFile: (activity: Activity) => void;
  onReveal: (activity: Activity) => Promise<unknown>;
  onTransferAction: (
    activity: Activity,
    action: 'pause_transfer' | 'resume_transfer' | 'cancel_transfer',
  ) => void;
}
export function ActivityList({
  activities,
  peers,
  onError,
  onReveal,
  onDragFile,
  onTransferAction,
}: Props) {
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
              <FileCard
                activity={activity}
                route={route}
                statusLabel={statuses[activity.status] ?? activity.status}
                onDragFile={onDragFile}
              />
            )}
            {incoming &&
              activity.kind !== 'text' &&
              activity.status === 'completed' &&
              activity.path && (
                <div className="transfer-actions">
                  <button
                    className="button secondary small"
                    onClick={() => {
                      void onReveal(activity).catch((error) => onError(String(error)));
                    }}
                  >
                    <FolderOpen size={14} />
                    打开所在位置
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
