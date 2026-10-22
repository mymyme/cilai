import { writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import channels from '../shell/update-channels.js';

export async function writeUpdateConfiguration({ resources, platform, arch, profile, channel, distribution }) {
  const configuration = channels.channelConfiguration({ platform, arch, buildProfile: profile,
    buildChannel: channel, distribution });
  await writeFile(join(resources, 'update-channels.json'), JSON.stringify(configuration, null, 2) + '\n');
  if (distribution === 'local') {
    await rm(join(resources, 'app-update.yml'), { force: true });
    await rm(join(resources, 'portable-update.json'), { force: true });
  } else if (distribution === 'portable') {
    await writeFile(join(resources, 'portable-update.json'), JSON.stringify({ schemaVersion: 1,
      kind: 'cilai-portable', platform, arch, feed: configuration.channels.stable }, null, 2) + '\n');
  } else {
    // Signed installer packages already have publisher metadata. Keep it intact;
    // an unpacked macOS package needs the same native updater configuration.
    const file = join(resources, 'app-update.yml');
    try { await readFile(file); }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      await writeFile(file, `provider: generic\nurl: ${configuration.channels.stable}\nchannel: latest\nupdaterCacheDirName: cilai-personal-workbench-updater\n`);
    }
  }
  return configuration;
}
