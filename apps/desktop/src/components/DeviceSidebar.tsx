import { useState } from 'react';
import { ArrowDownUp, ArrowRight, Laptop, Monitor, Plus, ShieldCheck, Wifi } from 'lucide-react';
import { version } from '../../package.json';
import { PairedDeviceList, type PairedDeviceListProps } from './PairedDeviceList';

interface Props extends PairedDeviceListProps {
  manual: boolean;
  connecting: boolean;
  onPair: (addresses: string | string[]) => void;
}

export function DeviceSidebar({
  snapshot,
  selected,
  manual,
  connecting,
  onRefresh,
  onSelect,
  onToggleManual,
  onDelete,
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
        <span
          className="version"
          title={`当前版本 v${version}`}
          aria-label={`当前版本 v${version}`}
        >
          v{version}
        </span>
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
      <PairedDeviceList
        key={JSON.stringify([snapshot?.network.mode, snapshot?.network.relayUrl])}
        snapshot={snapshot}
        selected={selected}
        onRefresh={onRefresh}
        onSelect={onSelect}
        onDelete={onDelete}
        onToggleManual={onToggleManual}
      />
      {unpaired.length > 0 && snapshot?.network.mode !== 'internet' && (
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
      {manual && snapshot?.network.mode === 'internet' && (
        <p className="sidebar-note">
          当前为仅互联网模式，请在顶部“互联网设置与跨网配对”输入对方的配对码。
        </p>
      )}
      {manual && snapshot?.network.mode !== 'internet' && (
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
