import assert from 'node:assert/strict';
import test from 'node:test';
import { sendPaths } from '../src/hooks/sendPaths.ts';

test('a mixed external drop queues every path, including items after the fourth', async () => {
  const paths = [
    '/sample/a.txt',
    '/sample/folder',
    '/sample/empty-folder',
    '/sample/b.bin',
    '/sample/c.bin',
    '/sample/d',
    '/sample/e',
  ];
  const calls: Record<string, unknown>[] = [];
  await sendPaths(
    [...paths, paths[0]],
    'chosen-peer',
    async <T>(command: string, args?: Record<string, unknown>) => {
      assert.equal(command, 'send_path');
      calls.push(args!);
      return 'task-id' as T;
    },
    (message) => assert.fail(message),
  );
  assert.deepEqual(
    calls.map((args) => args.path),
    paths,
  );
  assert.ok(calls.every((args) => args.peerId === 'chosen-peer'));
});

test('a rejected path reports its name and does not discard later paths', async () => {
  const accepted: string[] = [];
  const errors: string[] = [];
  await sendPaths(
    ['/sample/a', '/sample/denied', '/sample/folder'],
    'chosen-peer',
    async <T>(_command: string, args?: Record<string, unknown>) => {
      if (args!.path === '/sample/denied') throw new Error('access denied');
      accepted.push(String(args!.path));
      return 'task-id' as T;
    },
    (message) => errors.push(message),
  );
  assert.deepEqual(accepted, ['/sample/a', '/sample/folder']);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /\/sample\/denied.*access denied/);
});
