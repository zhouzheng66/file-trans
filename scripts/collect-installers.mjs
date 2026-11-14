import fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseVersion } from './release-version.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [platform, target] = process.argv.slice(2);
const configurations = {
  'windows-x64': { target: 'x86_64-pc-windows-msvc', formats: [['nsis', '-setup.exe']] },
  'macos-arm64': { target: 'aarch64-apple-darwin', formats: [['dmg', '.dmg']] },
  'macos-x64': { target: 'x86_64-apple-darwin', formats: [['dmg', '.dmg']] },
  'linux-x64': {
    target: 'x86_64-unknown-linux-gnu',
    formats: [
      ['deb', '.deb'],
      ['appimage', '.AppImage'],
    ],
  },
};
const configuration = configurations[platform];
if (!configuration || configuration.target !== target)
  throw new Error('安装包平台与目标架构不匹配。');
const version = await releaseVersion();
const output = path.join(root, 'release-assets', platform);
await fs.mkdir(output, { recursive: true });
const prefix = `file-trans_${version}_${platform}`;
const bundles = [];
for (const [directory, suffix] of configuration.formats) {
  const bundleDir = path.join(root, 'target', target, 'release', 'bundle', directory);
  const files = (await fs.readdir(bundleDir, { withFileTypes: true })).filter(
    (entry) => entry.isFile() && entry.name.endsWith(suffix),
  );
  if (files.length !== 1)
    throw new Error(`${platform} 应生成一个 ${suffix} 安装包，实际为 ${files.length}。`);
  const filename = `${prefix}${suffix}`;
  const destination = path.join(output, filename);
  await fs.copyFile(path.join(bundleDir, files[0].name), destination);
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(destination)) hash.update(chunk);
  const sha256 = hash.digest('hex');
  await fs.writeFile(`${destination}.sha256`, `${sha256}  ${filename}\n`, 'ascii');
  bundles.push({ filename, sha256, bytes: (await fs.stat(destination)).size });
}
await fs.writeFile(
  path.join(output, `${prefix}.json`),
  JSON.stringify(
    {
      version,
      platform,
      target,
      commit: process.env.GITHUB_SHA ?? null,
      bundles,
    },
    null,
    2,
  ) + '\n',
);
console.log(JSON.stringify({ platform, bundles }, null, 2));
