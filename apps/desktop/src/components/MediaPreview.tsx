import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Activity } from '../types';

interface Props {
  activity: Activity;
  kind: 'image' | 'video';
}

export function MediaPreview({ activity, kind }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const [nearby, setNearby] = useState(false);
  const [url, setUrl] = useState('');
  const [error, setError] = useState('');
  const unavailable = kind === 'image' ? '图片预览不可用' : '视频预览不可用';
  const { id, peerId, direction, content } = activity;

  useEffect(() => {
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return;
        setNearby(true);
        observer.disconnect();
      },
      { root: container.current?.closest('.conversation'), rootMargin: '200px' },
    );
    if (container.current) observer.observe(container.current);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!nearby) return;
    let active = true;
    void invoke<string>('prepare_media_preview', { key: { id, peerId, direction } })
      .then((source) => {
        if (active) setUrl(source);
      })
      .catch((reason) => {
        if (active) setError(typeof reason === 'string' && reason ? reason : unavailable);
      });
    return () => {
      active = false;
    };
  }, [nearby, id, peerId, direction, unavailable]);

  return (
    <div className={error ? 'preview-error' : 'media-preview'} ref={container}>
      {error ? (
        <span>{error}</span>
      ) : (
        url &&
        (kind === 'image' ? (
          <img
            src={url}
            alt={content}
            draggable={false}
            decoding="async"
            onError={() => setError(unavailable)}
          />
        ) : (
          <video
            src={url}
            controls
            playsInline
            preload="metadata"
            aria-label={content}
            title={content}
            onError={() => setError(unavailable)}
          />
        ))
      )}
    </div>
  );
}
