import { Pause, Play, X } from 'lucide-react';
import type { Activity } from '../types';

interface Props {
  activity: Activity;
  onAction: (
    activity: Activity,
    action: 'pause_transfer' | 'resume_transfer' | 'cancel_transfer',
  ) => void;
}

export function TransferActions({ activity, onAction }: Props) {
  if (!activity.resumable || ['completed', 'cancelled', 'rejected'].includes(activity.status))
    return null;
  const resumable = ['paused', 'failed'].includes(activity.status);
  return (
    <div className="transfer-actions">
      {resumable ? (
        <button
          className="button secondary small"
          onClick={() => onAction(activity, 'resume_transfer')}
        >
          <Play size={12} />
          恢复
        </button>
      ) : (
        <button
          className="button secondary small"
          onClick={() => onAction(activity, 'pause_transfer')}
        >
          <Pause size={12} />
          暂停
        </button>
      )}
      <button
        className="button secondary small"
        onClick={() => onAction(activity, 'cancel_transfer')}
      >
        <X size={12} />
        取消任务
      </button>
    </div>
  );
}
