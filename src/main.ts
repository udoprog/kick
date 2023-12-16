import * as core from '@actions/core';
import * as tc from '@actions/tool-cache';
import * as httpm from '@actions/http-client';
import * as fs from 'fs';
import * as path from 'path';

const USER_AGENT = 'udoprog/kick-action';

const IS_WINDOWS = process.platform === 'win32'
const IS_MAC = process.platform === 'darwin'

async function version(repo: string, key: string): Promise<string> {
    const version = core.getInput(key);

    if (version !== 'latest') {
        return version;
    }

    core.info(`Searching the latest version of ${repo} ...`);

    const http = new httpm.HttpClient(USER_AGENT, [], {
        allowRetries: false
    });

    const response = await http.get(`https://api.github.com/repos/${repo}/releases/latest`);
    const body = await response.readBody();
    return Promise.resolve(JSON.parse(body).tag_name);
}

/**
 * Download and return the path to an executable kick tool.
 *
 * @param tag The tag to download.
 */
async function download(tag: string): Promise<{ path: string, dir: string }> {
    let platform;
    let ext = '';

    if (IS_WINDOWS) {
        platform = 'x86_64-windows.exe';
        ext = '.exe';
    } else if (IS_MAC) {
        throw new Error("macOS is not supported");
    } else {
        platform = 'x86_64-linux';
    }

    const name = `kick-${platform}`;
    const url = `https://github.com/udoprog/kick/releases/download/${tag}/${name}`;
    let toolPath = await tc.downloadTool(url);
    let dir = path.dirname(toolPath);
    let newName = path.join(dir, `kick${ext}`);
    fs.renameSync(toolPath, newName);

    if (!IS_WINDOWS) {
        fs.chmodSync(newName, '755');
    }

    return { path: newName, dir };
}

async function innerMain() {
    const tag = await version('udoprog/kick', 'version') || process.env.GITHUB_ACTION_REF;

    if (!tag) {
        throw new Error("No version found or specified");
    }

    core.info(`Downloading 'kick' from tag '${tag}'`);
    const tool = await download(tag);

    if (!!process.env.GITHUB_PATH) {
        fs.writeFileSync(process.env.GITHUB_PATH, `${tool.dir}\n`);
    }

    core.info(`Downloaded to ${tool.path} and added ${tool.dir} to GITHUB_PATH`);
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
