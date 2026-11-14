import { useEffect, useState } from 'react';
import { ask, open } from '@tauri-apps/plugin-dialog';
import { ArrowDownUp, FolderOpen, Link2, ShieldCheck, X } from 'lucide-react';
import { useEngine } from './hooks/useEngine';
import { useDropFiles } from './hooks/useDropFiles';
import { sendPaths } from './hooks/sendPaths';
import { PairingDialog } from './components/PairingDialog';
import { ActivityList } from './components/ActivityList';
import { DeviceSidebar } from './components/DeviceSidebar';
import { Composer } from './components/Composer';
import { Welcome } from './components/Welcome';
import { NetworkPanel } from './components/NetworkPanel';
import './styles.css';

function ignore(promise: Promise<unknown>) {
  void promise.catch(() => {});
}

export default function App() {
  const { snapshot, error, setError, run } = useEngine();
  const [selected, setSelected] = useState<string | null>(null);
  const [manual, setManual] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const dragging = useDropFiles(selected, run, setError);
  const peer = snapshot?.peers.find((peer) => peer.id === selected);
  const pairing = snapshot?.pairings.find((pairing) =>
    ['pending', 'waiting_peer'].includes(pairing.status),
  );
  const activities =
    snapshot?.activities.filter((activity) => activity.peerId === peer?.id).reverse() ?? [];
  const pendingElsewhere =
    snapshot?.activities.filter(
      (activity) =>
        activity.direction === 'incoming' &&
        activity.status === 'awaiting_accept' &&
        activity.peerId !== selected,
    ) ?? [];

  useEffect(() => {
    if (snapshot && !snapshot.peers.some((peer) => peer.id === selected))
      setSelected(snapshot.peers[0]?.id ?? null);
  }, [snapshot, selected]);

  async function pair(addresses: string | string[]) {
    setConnecting(true);
    try {
      for (const address of typeof addresses === 'string' ? [addresses] : addresses) {
        try {
          await run('pair_device', { address });
          setError(null);
          setManual(false);
          return;
        } catch {
          /* 多网卡设备继续尝试下一个地址；全部失败时保留最后错误。 */
        }
      }
    } finally {
      setConnecting(false);
    }
  }
  async function choosePaths(directory: boolean) {
    if (!peer) return;
    const peerId = peer.id;
    try {
      const paths = await open({
        multiple: !directory,
        directory,
        title: directory ? '选择要发送的文件夹' : '选择要发送的文件',
      });
      if (paths)
        await sendPaths(typeof paths === 'string' ? [paths] : paths, peerId, run, setError);
    } catch (error) {
      setError(String(error));
    }
  }
  async function chooseDirectory() {
    try {
      const path = await open({ directory: true, multiple: false, title: '选择文件接收目录' });
      if (path) await run('set_download_dir', { path });
    } catch (error) {
      setError(String(error));
    }
  }
  async function forgetPeer(id: string, name: string) {
    try {
      if (
        await ask(`移除对 ${name} 的信任？后续收发需要重新配对。`, {
          title: '移除设备信任',
          kind: 'warning',
          okLabel: '移除',
          cancelLabel: '取消',
        })
      )
        await run('forget_peer', { id });
    } catch (error) {
      setError(String(error));
    }
  }

  return (
    <div className="app-shell">
      <DeviceSidebar
        snapshot={snapshot}
        selected={selected}
        manual={manual}
        connecting={connecting}
        onSelect={setSelected}
        onToggleManual={() => setManual(!manual)}
        onPair={(addresses) => ignore(pair(addresses))}
      />
      <main className={`main-panel ${dragging ? 'dragging' : ''}`}>
        <header className="main-header">
          <div>
            <div className="eyebrow">让文件，在你的电脑间流动</div>
            <h1>{peer?.name ?? '你的电脑，轻松相连'}</h1>
          </div>
          <span className="network-badge">设备间加密传输</span>
        </header>
        <NetworkPanel snapshot={snapshot} run={run} onError={setError} />
        {error && (
          <div className="error-banner" role="alert">
            <span>{error}</span>
            <button
              className="icon-button"
              aria-label="关闭错误提示"
              onClick={() => setError(null)}
            >
              <X size={16} />
            </button>
          </div>
        )}
        {snapshot?.discoveryError && <div className="notice-banner">{snapshot.discoveryError}</div>}
        {snapshot?.pairings
          .filter((pairing) => pairing.status === 'failed')
          .slice(-1)
          .map((pairing) => (
            <div className="notice-banner" key={pairing.id}>
              配对未完成：{pairing.error}
            </div>
          ))}
        {snapshot?.pairings
          .filter((pairing) => pairing.status === 'rejected')
          .slice(-1)
          .map((pairing) => (
            <div className="notice-banner" key={pairing.id}>
              与 {pairing.peer.name} 的配对已取消。
            </div>
          ))}
        {pendingElsewhere.length > 0 && (
          <div className="notice-banner">
            <button className="text-button" onClick={() => setSelected(pendingElsewhere[0].peerId)}>
              另有 {pendingElsewhere.length} 个传输任务等待接收，点击查看
            </button>
          </div>
        )}
        <div className="conversation">
          {peer ? (
            <>
              <div className="conversation-top">
                <span>
                  <ShieldCheck size={14} />
                  已信任的设备 · 文字与文件原样传送
                </span>
                <button
                  className="text-button"
                  onClick={() => ignore(forgetPeer(peer.id, peer.name))}
                >
                  移除信任
                </button>
              </div>
              {activities.length > 0 ? (
                <ActivityList
                  activities={activities}
                  peers={snapshot!.peers}
                  onDecide={(activity, accepted) =>
                    ignore(
                      run('decide_file', { id: activity.id, peerId: activity.peerId, accepted }),
                    )
                  }
                  onError={setError}
                  onTransferAction={(activity, command) =>
                    ignore(
                      run(command, {
                        key: {
                          id: activity.id,
                          peerId: activity.peerId,
                          direction: activity.direction,
                        },
                      }),
                    )
                  }
                />
              ) : (
                <div className="conversation-empty">
                  <Link2 size={34} />
                  <h2>已经准备好，发点什么吧</h2>
                  <p>发送文字，或从系统拖入文件和文件夹。</p>
                </div>
              )}
            </>
          ) : (
            <Welcome onAdd={() => setManual(true)} />
          )}
        </div>
        {peer && (
          <Composer
            key={peer.id}
            onSend={(text) => run('send_text', { peerId: peer.id, text })}
            onChooseFile={() => ignore(choosePaths(false))}
            onChooseFolder={() => ignore(choosePaths(true))}
          />
        )}
        <footer className="main-footer">
          <button
            className="text-button download-location"
            disabled={!snapshot}
            title={snapshot?.downloadDir}
            onClick={() => ignore(chooseDirectory())}
          >
            <FolderOpen size={14} />
            <span>接收目录：{snapshot?.downloadDir ?? '正在读取…'}</span>
          </button>
          <span>双方在线即可收发</span>
        </footer>
        {dragging && (
          <div className="drop-overlay">
            <ArrowDownUp size={45} />
            <h2>松开，发送文件或文件夹</h2>
            <p>{peer?.name ?? '请先配对并选择接收设备'}</p>
          </div>
        )}
      </main>
      {pairing && (
        <PairingDialog
          pairing={pairing}
          onDecide={(id, accepted) => ignore(run('decide_pairing', { id, accepted }))}
        />
      )}
    </div>
  );
}
