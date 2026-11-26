import { useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { transferKey } from '../attachments';
import type { Activity } from '../types';
import { sendPaths } from './sendPaths';

type Run = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function useDropFiles(
  selected: string | null,
  run: Run,
  onError: (message: string) => void,
) {
  const [dragging, setDragging] = useState(false);
  const draggingFromChat = useRef(false);
  async function startFileDrag(activity: Activity) {
    if (draggingFromChat.current) return;
    draggingFromChat.current = true;
    try {
      await invoke('start_file_drag', { key: transferKey(activity) });
    } catch (error) {
      onError(String(error));
    } finally {
      draggingFromChat.current = false;
    }
  }
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let enteredFromChat = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (disposed) return;
        if (event.payload.type === 'enter') enteredFromChat = draggingFromChat.current;
        // 原生拖动结束通知与最后一个 drop 可能先后到达，按 enter 记录来源。
        if (draggingFromChat.current || enteredFromChat) {
          setDragging(false);
          return;
        }
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
  return { dragging, startFileDrag };
}
