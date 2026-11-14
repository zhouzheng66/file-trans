import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen } from 'lucide-react';

interface Props {
  path?: string;
  onChange: (path: string) => Promise<unknown>;
  onError: (message: string) => void;
}

export function ReceiveDirectory({ path, onChange, onError }: Props) {
  const [busy, setBusy] = useState(false);
  async function choose() {
    if (busy || !path) return;
    setBusy(true);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: path,
        title: '选择文件和文件夹的接收目录',
      });
      if (selected) await onChange(selected);
    } catch (error) {
      onError(String(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <footer className="main-footer">
      <div className="download-location">
        <FolderOpen size={16} />
        <span title={path}>接收目录：{path ?? '正在读取…'}</span>
        <button
          className="button secondary small"
          disabled={!path || busy}
          title="修改后用于新接收的文件和文件夹，进行中的任务保留原目录"
          onClick={() => void choose()}
        >
          {busy ? '正在修改…' : '修改目录'}
        </button>
      </div>
      <span>已配对设备自动接收</span>
    </footer>
  );
}
