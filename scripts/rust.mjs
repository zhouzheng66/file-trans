import { spawn } from 'node:child_process';
import { root, toolchainEnv } from './toolchain.mjs';

const child = spawn('cargo', process.argv.slice(2), {
  cwd: root,
  env: toolchainEnv(),
  stdio: 'inherit',
});
child.on('error', (error) => {
  console.error(`无法运行 Cargo，请先安装 Rust：${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code) => {
  process.exitCode = code ?? 1;
});
