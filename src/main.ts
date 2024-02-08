import * as core from '@actions/core';
import * as tc from '@actions/tool-cache';

const IS_WINDOWS = process.platform === 'win32'
const IS_MAC = process.platform === 'darwin'

async function download(tag: string): Promise<string> {
    let platform;
    let ext = 'tar.gz';
    let zip = false;
    let arch = 'x86';

    if (process.arch === 'x64') {
        arch = 'x86_64';
    }

    if (IS_WINDOWS) {
        platform = 'windows';
        ext = 'zip';
        zip = true;
    } else if (IS_MAC) {
        platform = 'macos';
    } else {
        platform = 'linux';
    }

    let name = `kick-${tag}-${arch}-${platform}.${ext}`;

    const url = `https://github.com/udoprog/kick/releases/download/${tag}/${name}`;

    core.info(`Downloading ${url}`);
    let directory = await tc.downloadTool(url);

    if (zip) {
        return await tc.extractZip(directory);
    } else {
        return await tc.extractTar(directory);
    }
}

async function innerMain() {
    const tag = await core.getInput("version");

    if (!tag) {
        throw new Error("Could not determine version of kick to use");
    }

    core.info(`Downloading 'kick' from tag '${tag}'`);
    core.addPath(await download(tag));
}

async function main() {
    try {
        await innerMain();
    } catch (error) {
        // @ts-ignore
        core.setFailed(error.message);
    }
}

main();
