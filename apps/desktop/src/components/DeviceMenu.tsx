import { useEffect, useRef, useState } from 'react';
import { MoreHorizontal, Trash2 } from 'lucide-react';

interface Props {
  name: string;
  onDelete: () => void;
}

export function DeviceMenu({ name, onDelete }: Props) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const item = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    item.current?.focus();
    function closeOutside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener('pointerdown', closeOutside);
    return () => document.removeEventListener('pointerdown', closeOutside);
  }, [open]);

  return (
    <div
      className="device-menu"
      ref={root}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          setOpen(false);
          trigger.current?.focus();
        }
      }}
    >
      <button
        ref={trigger}
        className="icon-button device-menu-trigger"
        aria-label={`${name} 的设备菜单`}
        aria-haspopup="menu"
        aria-expanded={open}
        title="设备菜单"
        onClick={() => setOpen(!open)}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <MoreHorizontal size={17} />
      </button>
      {open && (
        <div className="device-menu-popup" role="menu" aria-label={`${name} 的设备操作`}>
          <button
            ref={item}
            type="button"
            role="menuitem"
            className="device-menu-delete"
            onClick={() => {
              setOpen(false);
              trigger.current?.focus();
              onDelete();
            }}
          >
            <Trash2 size={14} />
            删除设备
          </button>
        </div>
      )}
    </div>
  );
}
