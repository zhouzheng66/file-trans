import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export const root = fileURLToPath(new URL('../', import.meta.url));
export function toolchainEnv() {
  const cargo = path.join(root, '.tooling/cargo');
  const rustup = path.join(root, '.tooling/rustup');
  const binary = process.platform === 'win32' ? 'cargo.exe' : 'cargo';
  if (!existsSync(path.join(cargo, 'bin', binary))) return process.env;
  return {
    ...process.env,
    CARGO_HOME: cargo,
    RUSTUP_HOME: rustup,
    PATH: `${path.join(cargo, 'bin')}${path.delimiter}${process.env.PATH ?? ''}`,
  };
}
