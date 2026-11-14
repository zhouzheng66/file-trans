import { ShieldCheck, X } from 'lucide-react';
import type { Pairing } from '../types';

interface Props {
  pairing: Pairing;
  onDecide: (id: string, accepted: boolean) => void;
}
export function PairingDialog({ pairing, onDecide }: Props) {
  const waiting = pairing.status === 'waiting_peer';
  return (
    <div className="modal-backdrop">
      <section
        className="pairing-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="pair-title"
      >
        <div className="dialog-icon">
          <ShieldCheck size={28} />
        </div>
        <h2 id="pair-title">确认是你的电脑</h2>
        <p>
          <strong>{pairing.peer.name}</strong>{' '}
          {pairing.direction === 'incoming' ? '请求与你配对' : '正在等待配对'}。
        </p>
        <p className="muted">请核对两台电脑显示的安全校验码一致。</p>
        <div className="security-code">{pairing.code}</div>
        <p className="fingerprint">设备指纹 {pairing.peer.id.slice(0, 16)}…</p>
        {waiting ? (
          <p className="waiting">已提交选择，等待对方确认…</p>
        ) : (
          <div className="dialog-actions">
            <button className="button secondary" onClick={() => onDecide(pairing.id, false)}>
              <X size={16} />
              拒绝
            </button>
            <button className="button primary" autoFocus onClick={() => onDecide(pairing.id, true)}>
              <ShieldCheck size={16} />
              一致，记住设备
            </button>
          </div>
        )}
      </section>
    </div>
  );
}
