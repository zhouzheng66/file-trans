import { useState } from 'react';
import {
  ArrowDownUp,
  ArrowRight,
  ChevronRight,
  Laptop,
  Monitor,
  Plus,
  ShieldCheck,
  Wifi,
} from 'lucide-react';
import type { Snapshot } from '../types';

interface Props {
  snapshot: Snapshot | null;
  selected: string | null;
  manual: boolean;
  connecting: boolean;
  onSelect: (id: string) => void;
  onToggleManual: () => void;
  onPair: (addresses: string | string[]) => void;
}

export function DeviceSidebar({
  snapshot,
  selected,
  manual,
  connecting,
  onSelect,
  onToggleManual,
  onPair,
}: Props) {
  const [address, setAddress] = useState('');
  const unpaired =
    snapshot?.discovered.filter(
      (device) => !snapshot.peers.some((peer) => peer.id === device.id),
    ) ?? [];
  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark">
          <ArrowDownUp size={22} />
        </span>
        <div>
          <strong>轻传</strong>
          <span>FILE TRANS</span>
        </div>
        <span className="version">LOCAL</span>
      </div>
      <div className="local-device">
        <div className="local-device-icon">
          <Laptop size={22} />
        </div>
        <div>
          <strong title={snapshot?.local.name}>{snapshot?.local.name ?? '正在启动…'}</strong>
          <span>此电脑 · 无需登录</span>
        </div>
      </div>
      <div className="section-title">
        <span>我的设备</span>
        <button
          className="icon-button"
          aria-label="手动添加设备"
          title="手动添加设备"
          onClick={onToggleManual}
        >
          <Plus size={17} />
        </button>
      </div>
      <div className="peer-list">
        {snapshot?.peers.map((device) => {
          const discovered = snapshot.discovered.some((found) => found.id === device.id);
          return (
            <button
              className={`peer-button ${selected === device.id ? 'selected' : ''}`}
              key={device.id}
              onClick={() => onSelect(device.id)}
            >
              <Monitor size={22} />
              <span>
                <strong title={device.name}>{device.name}</strong>
                <small>
                  <i className={discovered ? 'dot online' : 'dot'} />
                  {discovered ? '局域网已发现' : '已配对 · 可尝试连接'}
                </small>
              </span>
              <ChevronRight size={15} />
            </button>
          );
        })}
        {snapshot?.peers.length === 0 && (
          <p className="sidebar-note">
            添加另一台电脑，
            <br />
            开始你的第一次传输。
          </p>
        )}
      </div>
      {unpaired.length > 0 && (
        <>
          <div className="section-title">
            <span>附近的设备</span>
            <Wifi size={14} />
          </div>
          {unpaired.map((device) => (
            <button
              className="peer-button nearby"
              disabled={connecting}
              key={device.id}
              onClick={() => onPair(device.addresses)}
            >
              <Monitor size={21} />
              <span>
                <strong>{device.name}</strong>
                <small>点击配对</small>
              </span>
              <Plus size={15} />
            </button>
          ))}
        </>
      )}
      {manual && (
        <form
          className="manual-form"
          onSubmit={(event) => {
            event.preventDefault();
            onPair(address);
          }}
        >
          <label htmlFor="manual-address">对方的局域网地址</label>
          <input
            id="manual-address"
            autoFocus
            placeholder="192.168.1.20:45678"
            value={address}
            onChange={(event) => setAddress(event.target.value)}
          />
          <button className="button primary small" disabled={!address.trim() || connecting}>
            {connecting ? '正在连接…' : '连接并配对'}
            <ArrowRight size={14} />
          </button>
        </form>
      )}
      <div className="sidebar-bottom">
        <div>
          <ShieldCheck size={15} />
          <span>配对设备间加密传输</span>
        </div>
        <p>本机连接地址</p>
        {snapshot?.localAddresses.map((address) => (
          <code key={address}>{address}</code>
        ))}
        {snapshot?.localAddresses.length === 0 && <small>暂无可用的局域网 IPv4 地址</small>}
      </div>
    </aside>
  );
}
