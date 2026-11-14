type Run = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** 文件选择与系统原生拖拽共用同一后端入口，逐项反馈错误。 */
export async function sendPaths(
  paths: string[],
  peerId: string,
  run: Run,
  onError: (message: string) => void,
) {
  const unique = [...new Set(paths)];
  if (unique.length > 128) {
    onError('一次最多添加 128 个文件或文件夹，请分批操作。');
    return;
  }
  const errors: string[] = [];
  for (const path of unique) {
    try {
      await run('send_path', { peerId, path });
    } catch (error) {
      errors.push(`${path}：${String(error)}`);
    }
  }
  if (errors.length) onError(errors.join('\n'));
}
