import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const readJson = async (name) => JSON.parse(await fs.readFile(path.join(root, name), 'utf8'));
export async function releaseVersion() {
  const config = await readJson('apps/desktop/src-tauri/tauri.conf.json');
  const packages = await Promise.all(['package.json', 'apps/desktop/package.json'].map(readJson));
  const cargo = await fs.readFile(path.join(root, 'Cargo.toml'), 'utf8');
  const cargoVersion = cargo.match(/\[workspace\.package\][\s\S]*?^version = "([^"]+)"/m)?.[1];
  if (packages.some((item) => item.version !== config.version) || cargoVersion !== config.version)
    throw new Error('发布版本不一致：请同步 Cargo、npm 与 Tauri 配置。');
  if (process.env.GITHUB_REF_TYPE === 'tag' && process.env.GITHUB_REF_NAME !== `v${config.version}`)
    throw new Error('Git 标签必须与安装包版本一致。');
  return config.version;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  console.log(`File Trans v${await releaseVersion()}`);
