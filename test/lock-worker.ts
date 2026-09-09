import { readFile } from 'node:fs/promises';
import { withFileLock, writeAtomic } from '../src/host/store/fileLock';

// One "host" of the fileLock test: read → increment → write a JSON counter N times under the lock. Without the lock, hosts running
// this side by side lose increments to each other's stale reads
const [file, times] = [process.argv[2]!, Number(process.argv[3])];
for (let i = 0; i < times; i++) {
  await withFileLock(file, async () => {
    const n = await readFile(file, 'utf8').then(Number, () => 0);
    await new Promise(r => setTimeout(r, 1));
    await writeAtomic(file, String(n + 1));
  });
}
