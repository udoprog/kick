import * as core from '@actions/core';
import * as tc from '@actions/tool-cache';
import * as httpm from '@actions/http-client';

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

    if (response.message.statusCode !== 200) {
        throw new Error(`Failed to fetch latest version of ${repo}: ${response.message.statusCode}: ${body}`);
    }

    let data = JSON.parse(body);

    if (!data.tag_name) {
        throw new Error(`Failed to fetch latest version of ${repo}: No tag_name`);
    }

    return data.tag_name;
}

/**
 * Download and return the path to an executable kick tool.
 *
 * @param tag The tag to download.
 */
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
    const tag = await version('udoprog/kick', 'version') || process.env.GITHUB_ACTION_REF;

    if (!tag) {
        throw new Error("No version found or specified");
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