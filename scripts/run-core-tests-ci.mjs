import { spawn } from 'node:child_process';

// 保留完整控制台输出，并把失败尾部写入检查注释，便于从公开 CI 状态定位问题。
const child = spawn(
  process.platform === 'win32' ? 'npm.cmd' : 'npm',
  ['run', 'test:core', '--', '--locked'],
  {
    stdio: ['ignore', 'pipe', 'pipe'],
    shell: process.platform === 'win32',
  },
);
let tail = '';
function record(chunk, stream) {
  stream.write(chunk);
  tail = (tail + chunk.toString('utf8')).slice(-48_000);
}
child.stdout.on('data', (chunk) => record(chunk, process.stdout));
child.stderr.on('data', (chunk) => record(chunk, process.stderr));
child.on('error', (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on('close', (code) => {
  if (code !== 0 && process.env.GITHUB_ACTIONS === 'true') {
    const message = Buffer.from(
      tail
        .replace(/\u001b\[[0-9;]*m/g, '')
        .split('\n')
        .slice(-80)
        .join('\n'),
    )
      .subarray(-3000)
      .toString('utf8')
      .replaceAll('%', '%25')
      .replaceAll('\r', '%0D')
      .replaceAll('\n', '%0A');
    console.log(`::error title=Core integration tests failed::${message}`);
  }
  process.exitCode = code ?? 1;
});
