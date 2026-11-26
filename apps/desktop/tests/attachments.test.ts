import assert from 'node:assert/strict';
import test from 'node:test';
import { hasLocalAttachment, previewKind, transferKey } from '../src/attachments.ts';
import type { Activity } from '../src/types.ts';

const file: Activity = {
  id: 'transfer',
  peerId: 'peer',
  direction: 'incoming',
  kind: 'file',
  content: 'photo.jpg',
  status: 'completed',
  size: 10,
  transferred: 10,
  path: '/downloads/photo.jpg',
  error: null,
  timestamp: 0,
  route: 'lan',
  resumable: false,
  resumedFrom: 0,
  reconnectCount: 0,
};

test('local attachment actions require a saved incoming item or an outgoing source', () => {
  assert.ok(hasLocalAttachment(file));
  assert.ok(hasLocalAttachment({ ...file, kind: 'folder' }));
  assert.ok(hasLocalAttachment({ ...file, direction: 'outgoing', status: 'preparing' }));
  assert.equal(hasLocalAttachment({ ...file, status: 'verifying' }), false);
  assert.equal(hasLocalAttachment({ ...file, direction: 'outgoing', path: null }), false);
  assert.equal(hasLocalAttachment({ ...file, kind: 'text' }), false);
  assert.deepEqual(transferKey(file), { id: 'transfer', peerId: 'peer', direction: 'incoming' });
});

test('preview candidates use the final file extension regardless of case', () => {
  assert.equal(previewKind('C:\\Users\\sample\\旅行.JPEG'), 'image');
  assert.equal(previewKind('/sample/片段 (1).MOV'), 'video');
  assert.equal(previewKind('/sample.mov/README'), null);
  assert.equal(previewKind('/sample/photo.jpg.txt'), null);
  assert.equal(previewKind('/sample/drawing.svg'), null);
});
