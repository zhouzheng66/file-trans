import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { releaseVersion } from './release-version.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function run(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 120_000 });
  if (result.error || result.status !== 0)
    throw new Error(`${command} 失败：${result.error?.message ?? result.stderr ?? result.stdout}`);
  return result.stdout.trim();
}

async function verify() {
  const target = process.argv[2];
  const architecture = {
    'aarch64-apple-darwin': 'arm64',
    'x86_64-apple-darwin': 'x86_64',
  }[target];
  if (process.platform !== 'darwin' || !architecture)
    throw new Error('请在 macOS 上指定有效的 Apple 编译目标。');
  const version = await releaseVersion();
  let image = process.argv[3];
  if (!image) {
    const directory = path.join(root, 'target', target, 'release', 'bundle', 'dmg');
    const images = (await fs.readdir(directory)).filter((name) => name.endsWith('.dmg'));
    if (images.length !== 1) throw new Error(`预期一个 DMG，实际为 ${images.length}。`);
    image = path.join(directory, images[0]);
  }
  const mount = await fs.mkdtemp(path.join(os.tmpdir(), 'file-trans-dmg-check-'));
  let mounted = false;
  try {
    run('hdiutil', ['attach', path.resolve(image), '-readonly', '-nobrowse', '-mountpoint', mount]);
    mounted = true;
    const app = path.join(mount, 'File Trans.app');
    run('codesign', ['--verify', '--deep', '--strict', app]);
    const actual = run('plutil', [
      '-extract',
      'CFBundleShortVersionString',
      'raw',
      '-o',
      '-',
      path.join(app, 'Contents', 'Info.plist'),
    ]);
    if (actual !== version) throw new Error(`DMG 内应用版本错误：${actual}。`);
    const architectures = run('lipo', [
      '-archs',
      path.join(app, 'Contents', 'MacOS', 'file-trans-desktop'),
    ]);
    if (architectures !== architecture) throw new Error(`DMG 内应用架构错误：${architectures}。`);
    console.log(`已验证 DMG 内应用的完整签名、版本 ${version} 与架构 ${architecture}。`);
  } finally {
    if (mounted) run('hdiutil', ['detach', mount]);
    await fs.rmdir(mount);
  }
}

try {
  await verify();
} catch (error) {
  const message = String(error).slice(-3000);
  if (process.env.GITHUB_ACTIONS)
    console.error(
      `::error::${message.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A')}`,
    );
  else console.error(message);
  process.exitCode = 1;
}
