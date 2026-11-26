import type { Activity } from './types';

export function transferKey({ id, peerId, direction }: Activity) {
  return { id, peerId, direction };
}

export function hasLocalAttachment({ kind, direction, path, status }: Activity): boolean {
  return Boolean(
    path &&
    (kind === 'file' || kind === 'folder') &&
    (direction === 'outgoing' || (direction === 'incoming' && status === 'completed')),
  );
}

export function previewKind(path: string): 'image' | 'video' | null {
  if (/\.(jpe?g|png|webp|gif|bmp|avif)$/i.test(path)) return 'image';
  if (/\.(mp4|m4v|mov|webm|ogv)$/i.test(path)) return 'video';
  return null;
}
