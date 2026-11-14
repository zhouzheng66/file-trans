import { useEffect, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { isTauri } from '@tauri-apps/api/core';

type Run = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function useDropFiles(
  selected: string | null,
  run: Run,
  onError: (message: string) => void,
) {
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        setDragging(event.payload.type === 'over' || event.payload.type === 'enter');
        if (event.payload.type !== 'drop') return;
        if (!selected) {
          onError('请先选择一台已配对的电脑。');
          return;
        }
        for (const path of event.payload.paths.slice(0, 4))
          void run('send_file', { peerId: selected, path }).catch(() => {});
        if (event.payload.paths.length > 4) onError('一次最多同时发送 4 个文件，请分批拖入。');
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error) => onError(String(error)));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [run, selected, onError]);
  return dragging;
}
