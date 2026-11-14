import { useEffect, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { isTauri } from '@tauri-apps/api/core';
import { sendPaths } from './sendPaths';

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
        void sendPaths(event.payload.paths, selected, run, onError);
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
