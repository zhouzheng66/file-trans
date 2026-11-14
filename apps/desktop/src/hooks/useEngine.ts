import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Snapshot } from '../types';

export function useEngine() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(true);
  const refresh = useCallback(async () => {
    const value = await invoke<Snapshot>('get_snapshot');
    if (active.current) setSnapshot(value);
  }, []);
  useEffect(() => {
    active.current = true;
    if (!isTauri()) {
      setError('请在桌面客户端中使用。运行 npm run dev 启动 File Trans。');
      return;
    }
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        await refresh();
      } catch (error) {
        if (active.current) setError(String(error));
      }
      if (active.current) timer = setTimeout(poll, 800);
    };
    void poll();
    return () => {
      active.current = false;
      clearTimeout(timer);
    };
  }, [refresh]);
  const run = useCallback(
    async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      try {
        const result = await invoke<T>(command, args);
        await refresh();
        return result;
      } catch (error) {
        setError(String(error));
        throw error;
      }
    },
    [refresh],
  );
  return { snapshot, error, setError, run };
}
