use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use bstr::{BStr, ByteSlice};
use elf::abi::{EM_ALPHA, SHT_GNU_HASH, SHT_HASH};
use elf::endian::AnyEndian;
use elf::file::Class;
use elf::ElfStream;

use crate::process::Command;

pub(crate) fn find<P>(exe: P) -> Result<Vec<String>>
where
    P: AsRef<Path>,
{
    let exe = exe.as_ref();
    let header = elf_header(exe)?;

    let mut requires = find_requires_by_ldd(exe, header.marker())?;

    if header.sht_gnu_hash && !header.sht_hash {
        requires.push("rtld(GNU_HASH)".to_string());
    }

    Ok(requires)
}

// This method is copied and adapted from
// <https://github.com/cat-in-136/cargo-generate-rpm> under the MIT license
//
// Copyright (c) 2020 @cat_in_136
fn elf_header<P>(path: P) -> Result<ElfHeader>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let f = File::open(path).with_context(|| path.display().to_string())?;
    let stream = ElfStream::<AnyEndian, _>::open_stream(f).context("Open ELF stream")?;
    let headers = stream.section_headers();

    let machine = (stream.ehdr.class, stream.ehdr.e_machine);
    let sht_hash = headers.iter().any(|s| s.sh_type == SHT_HASH);
    let sht_gnu_hash = headers.iter().any(|s| s.sh_type == SHT_GNU_HASH);

    Ok(ElfHeader {
        machine,
        sht_hash,
        sht_gnu_hash,
    })
}

#[derive(Debug)]
struct ElfHeader {
    machine: (Class, u16),
    sht_hash: bool,
    sht_gnu_hash: bool,
}

impl ElfHeader {
    fn marker(&self) -> &'static [u8] {
        match self.machine {
            // alpha doesn't traditionally have 64bit markers
            (Class::ELF64, EM_ALPHA) | (Class::ELF64, 0x9026) => b"",
            (Class::ELF64, _) => b"(64bit)",
            (Class::ELF32, _) => b"",
        }
    }
}

fn find_requires_by_ldd(path: &Path, marker: &[u8]) -> Result<Vec<String>> {
    enum State {
        Initial,
        Header,
        Line(usize),
    }

    let mut process = Command::new("ldd")
        .arg("-v")
        .arg(path)
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdout = BufReader::new(process.stdout()?);

    let mut line = Vec::new();

    let mut requires = Vec::new();
    let mut state = State::Initial;

    loop {
        line.clear();

        let n = stdout.read_until(b'\n', &mut line)?;

        if n == 0 {
            break;
        }

        let prefix = line.iter().take_while(|b| b.is_ascii_whitespace()).count();
        let line = BStr::new(line.trim());

        match state {
            State::Initial => {
                if line == "Version information:" {
                    state = State::Header;
                    continue;
                }

                continue;
            }
            State::Header => {
                state = State::Line(prefix);
                continue;
            }
            State::Line(expected) => {
                // Skip over additional headers.
                if prefix == expected {
                    continue;
                }
            }
        };

        if line.is_empty() {
            continue;
        }

        let Some((lib, _)) = line.split_once_str(" => ") else {
            continue;
        };

        let Some((lib, version)) = lib.split_once_str(" ") else {
            continue;
        };

        if !accept_lib(lib) {
            continue;
        }

        if version == b"(GLIBC_PRIVATE)" {
            continue;
        }

        let mut line = lib.to_vec();
        line.extend(version);
        line.extend(marker);
        requires.push(BStr::new(&line).to_str_lossy().into_owned());
    }

    Ok(requires)
}

// This has been tweaked so that it matches roughly what find-requires emits.
fn accept_lib(lib: &[u8]) -> bool {
    if lib.starts_with(b"ld-linux-") {
        return true;
    }

    if let Some((head, _)) = lib.split_once_str("-") {
        if matches!(head, b"ld" | b"ld64") {
            return false;
        }
    }

    if let Some((head, _)) = lib.split_once_str(".") {
        if matches!(head, b"ld" | b"ld64") {
            return false;
        }
    }

    true
}
