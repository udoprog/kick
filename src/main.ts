import * as core from '@actions/core';
import * as tc from '@actions/tool-cache';

type Architecture = 'x86_64' | 'aarch64';
type Platform = 'windows' | 'macos' | 'linux';

interface Detect {
    platform: Platform;
    ext: string;
    method: (directory: string) => Promise<string>;
}

function detect_arch(): Architecture {
    if (process.arch === 'x64') {
        return 'x86_64';
    }

    if (process.arch === 'arm64') {
        return 'aarch64';
    }

    throw new Error(`Unsupported architecture \`${process.arch}\``);
}

function detect(): Detect {
    if (process.platform === 'win32') {
        return {platform: 'windows', ext: 'zip', method: tc.extractZip};
    }

    if (process.platform === 'darwin') {
        return {platform: 'macos', ext: 'tar.gz', method: tc.extractTar};
    }

    if (process.platform === 'linux') {
        return {platform: 'linux', ext: 'tar.gz', method: tc.extractTar};
    }

    throw new Error(`Unsupported platform \`${process.platform}\``);
}

async function download(tag: string): Promise<string> {
    const arch = detect_arch();
    const { platform, ext, method } = detect();
    const name = `kick-${tag}-${arch}-${platform}.${ext}`;
    const url = `https://github.com/udoprog/kick/releases/download/${tag}/${name}`;

    core.info(`Platform: ${platform}`);
    core.info(`Extension: ${ext}`);
    core.info(`Architecture: ${arch}`);
    core.info(`Name: ${name}`);
    core.info(`Url: ${url}`);

    core.info(`Downloading from ${url}`);
    const directory = await tc.downloadTool(url);
    core.info(`Extracting ${directory}`);
    return await method(directory);
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
