import { useState } from 'react';
import { Plus, Send, FolderOpen } from 'lucide-react';

interface Props {
  onSend: (text: string) => Promise<unknown>;
  onChooseFile: () => void;
  onChooseFolder: () => void;
}

export function Composer({ onSend, onChooseFile, onChooseFolder }: Props) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  async function send() {
    if (busy || !text.trim()) return;
    const sending = text;
    setBusy(true);
    try {
      await onSend(sending);
      setText((current) => (current === sending ? '' : current));
    } catch {
      /* 错误由桌面桥接显示，保留输入方便重试。 */
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="composer">
      <textarea
        placeholder="输入文字，Ctrl / ⌘ + Enter 发送…"
        value={text}
        onChange={(event) => setText(event.target.value)}
        maxLength={65536}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            void send();
          }
        }}
        aria-label="要发送的文字"
      />
      <div className="composer-actions">
        <button className="button secondary" onClick={onChooseFile}>
          <Plus size={16} />
          选择文件
        </button>
        <button className="button secondary" onClick={onChooseFolder}>
          <FolderOpen size={16} />
          选择文件夹
        </button>
        <span>可从系统直接拖入文件或文件夹</span>
        <button
          className="button primary send-button"
          disabled={busy || !text.trim()}
          onClick={() => void send()}
        >
          {busy ? '发送中…' : '发送文字'}
          <Send size={15} />
        </button>
      </div>
    </div>
  );
}
