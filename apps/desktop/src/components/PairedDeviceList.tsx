import { useState } from 'react';
import { Monitor, Plus, RefreshCw } from 'lucide-react';
import type { Snapshot } from '../types';
import { DeviceMenu } from './DeviceMenu';

export type ConnectionResults = Record<string, 'lan' | 'relay' | null>;
const labels = { lan: '局域网可达', relay: '中继可达', unreachable: '暂时无法连接' };

export interface PairedDeviceListProps {
  snapshot: Snapshot | null;
  selected: string | null;
  onSelect: (id: string) => void;
  onToggleManual: () => void;
  onDelete: (id: string, name: string) => void;
  onRefresh: () => Promise<ConnectionResults>;
}

export function PairedDeviceList({
  snapshot,
  selected,
  onSelect,
  onToggleManual,
  onDelete,
  onRefresh,
}: PairedDeviceListProps) {
  const [refreshing, setRefreshing] = useState(false);
  const [checks, setChecks] = useState<ConnectionResults>({});
  async function refresh() {
    setRefreshing(true);
    try {
      setChecks(await onRefresh());
    } catch {
      // 命令错误由 useEngine 展示。
    } finally {
      setRefreshing(false);
    }
  }

  return (
    <>
      <div className="section-title">
        <span>我的设备</span>
        <div className="device-list-actions">
          <button
            className="icon-button"
            aria-label="刷新连接状态"
            title="刷新连接状态"
            onClick={() => void refresh()}
            disabled={refreshing || !snapshot?.peers.length}
          >
            <RefreshCw size={15} />
          </button>
          <button
            className="icon-button"
            aria-label="手动添加设备"
            title="手动添加设备"
            onClick={onToggleManual}
          >
            <Plus size={17} />
          </button>
        </div>
      </div>
      <div className="peer-list">
        {snapshot?.peers.map((device) => {
          const discovered =
            snapshot.network.mode !== 'internet' &&
            snapshot.discovered.some((found) => found.id === device.id);
          const route = checks[device.id];
          const online = route === undefined ? discovered : Boolean(route);
          const status =
            route === undefined
              ? discovered
                ? '局域网已发现'
                : '已配对 · 可尝试连接'
              : labels[route ?? 'unreachable'];
          return (
            <div className="peer-row" key={device.id}>
              <button
                className={`peer-button ${selected === device.id ? 'selected' : ''}`}
                onClick={() => onSelect(device.id)}
              >
                <Monitor size={22} />
                <span>
                  <strong title={device.name}>{device.name}</strong>
                  <small title={route !== undefined ? '上次刷新结果' : undefined}>
                    <i className={!refreshing && online ? 'dot online' : 'dot'} />
                    {refreshing ? '正在检测…' : status}
                  </small>
                </span>
              </button>
              <DeviceMenu name={device.name} onDelete={() => onDelete(device.id, device.name)} />
            </div>
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
    </>
  );
}
