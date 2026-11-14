import { spawn } from 'node:child_process';
import path from 'node:path';
import { root, toolchainEnv } from './toolchain.mjs';

const child = spawn(
  process.platform === 'win32' ? 'npm.cmd' : 'npm',
  ['exec', '--', 'tauri', ...process.argv.slice(2)],
  {
    cwd: path.join(root, 'apps/desktop'),
    env: toolchainEnv(),
    stdio: 'inherit',
    shell: process.platform === 'win32',
  },
);
child.on('error', (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on('exit', (code) => {
  process.exitCode = code ?? 1;
});
