import { useEffect, useState } from 'react';
import { Cable, Copy, Globe2, Settings2 } from 'lucide-react';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import type { NetworkMode, Snapshot } from '../types';

interface Props {
  snapshot: Snapshot | null;
  run: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  onError: (message: string) => void;
}

const states: Record<string, string> = {
  online: '中继已连接',
  connecting: '正在连接中继',
  error: '中继连接失败，正在重试',
  disabled: '公网连接已关闭',
  unconfigured: '未配置中继，仍可局域网传输',
};

export function NetworkPanel({ snapshot, run, onError }: Props) {
  const [url, setUrl] = useState('');
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setUrl(snapshot?.network.relayUrl ?? '');
  }, [snapshot?.network.relayUrl]);
  const online = snapshot?.relay.state === 'online';
  const invite =
    snapshot?.relay.inviteExpiresAt && snapshot.relay.inviteExpiresAt > Date.now()
      ? snapshot.relay.inviteCode
      : null;
  async function action(command: string, args?: Record<string, unknown>) {
    setBusy(true);
    try {
      await run(command, args);
      if (command === 'pair_by_code') setCode('');
    } catch (error) {
      onError(String(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="network-panel" aria-label="网络设置">
      <div className="network-toolbar">
        <label htmlFor="network-mode">
          <Cable size={14} />
          传输模式
        </label>
        <select
          id="network-mode"
          disabled={!snapshot || busy}
          value={snapshot?.network.mode ?? 'auto'}
          onChange={(event) =>
            void action('set_network_settings', {
              settings: { ...snapshot!.network, mode: event.target.value as NetworkMode },
            })
          }
        >
          <option value="auto">自动 · 优先局域网</option>
          <option value="lan">仅局域网</option>
          <option value="internet">仅互联网中继</option>
        </select>
        <span
          className={`relay-state ${online ? 'online' : ''}`}
          title={snapshot?.relay.error ?? undefined}
        >
          <i className={`dot ${online ? 'online' : ''}`} />
          {states[snapshot?.relay.state ?? 'unconfigured']}
        </span>
      </div>
      <details className="network-details">
        <summary>
          <Settings2 size={13} />
          互联网设置与跨网配对
        </summary>
        <div className="network-settings-body">
          <form
            className="relay-url-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (snapshot)
                void action('set_network_settings', {
                  settings: { ...snapshot.network, relayUrl: url },
                });
            }}
          >
            <label htmlFor="relay-url">两端使用相同的中继地址</label>
            <div>
              <input
                id="relay-url"
                type="text"
                placeholder="wss://transfer.example.com"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
              />
              <button className="button secondary small" disabled={!snapshot || busy}>
                保存地址
              </button>
            </div>
          </form>
          {snapshot?.relay.error && <p className="inline-error">{snapshot.relay.error}</p>}
          <div className="remote-pairing">
            <div>
              <span>
                <Globe2 size={14} />
                让另一台电脑连接我
              </span>
              <button
                className="button secondary small"
                disabled={!online || busy}
                onClick={() => void action('create_invite')}
              >
                生成五分钟配对码
              </button>
              {invite && (
                <button
                  className="invite-code"
                  title="复制配对码"
                  onClick={() => {
                    void writeText(invite).catch((error) => onError(String(error)));
                  }}
                >
                  {invite}
                  <Copy size={13} />
                </button>
              )}
            </div>
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void action('pair_by_code', { code });
              }}
            >
              <label htmlFor="remote-code">连接另一台电脑</label>
              <div>
                <input
                  id="remote-code"
                  value={code}
                  placeholder="输入对方的跨网配对码"
                  maxLength={32}
                  onChange={(event) => setCode(event.target.value)}
                />
                <button className="button primary small" disabled={!online || busy || !code.trim()}>
                  连接
                </button>
              </div>
            </form>
          </div>
          <p className="network-hint">
            自动优先局域网，断网后从已保存断点重连。切换模式会让进行中的文件从断点切换通道，暂停任务保持暂停。跨网配对仍需两端核对安全校验码。
          </p>
        </div>
      </details>
    </section>
  );
}
