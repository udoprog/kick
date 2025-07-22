use std::fmt;
use std::io::{Cursor, Write};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};

use super::Architecture;

/// Builder of a debian archive.
pub struct Builder {
    control: ControlBuilder,
    data: DataBuilder,
}

impl Builder {
    pub(crate) fn new<N>(name: N, architecture: Architecture) -> Self
    where
        N: fmt::Display,
    {
        Self {
            control: ControlBuilder::new(name.to_string(), architecture),
            data: DataBuilder::new(),
        }
    }

    /// Set the description of the package.
    pub(crate) fn description<D>(&mut self, description: D) -> &mut Self
    where
        D: fmt::Display,
    {
        self.control.description = Some(description.to_string());
        self
    }

    /// Set the version of the package.
    pub(crate) fn version<V>(&mut self, version: V) -> &mut Self
    where
        V: fmt::Display,
    {
        self.control.version = Some(version.to_string());
        self
    }

    /// Add a file and return its associated builder.
    pub(crate) fn insert_file<P>(&mut self, path: P) -> &mut FileBuilder
    where
        P: AsRef<RelativePath>,
    {
        self.data.files.push(FileBuilder::new(path));
        self.data.files.last_mut().unwrap()
    }

    /// Insert a dependency and return its associated builder.
    pub(crate) fn insert_depends<N>(&mut self, name: N) -> &mut DependsBuilder
    where
        N: fmt::Display,
    {
        self.control.depends.push(DependsBuilder::new(name));
        self.control.depends.last_mut().unwrap()
    }

    /// Write the debian archive to the given path.
    pub(crate) fn write_to<O>(self, out: O) -> Result<()>
    where
        O: Write,
    {
        let contents = b"2.0\n";

        let header = ar::Header::new(b"debian-binary".to_vec(), contents.len() as u64);

        let mut builder = ar::Builder::new(out);
        builder
            .append(&header, Cursor::new(&contents))
            .context("Appending debian-binary")?;

        let control = self
            .control
            .build(&self.data.files)
            .context("Building control")?;
        let header = ar::Header::new(b"control.tar.xz".to_vec(), control.len() as u64);
        builder
            .append(&header, Cursor::new(&control))
            .context("Appending control")?;

        let data = self.data.build().context("Building data")?;
        let header = ar::Header::new(b"data.tar.xz".to_vec(), data.len() as u64);
        builder
            .append(&header, Cursor::new(&data))
            .context("Appending data")?;

        Ok(())
    }
}

pub(crate) struct ControlBuilder {
    name: String,
    architecture: Architecture,
    version: Option<String>,
    description: Option<String>,
    depends: Vec<DependsBuilder>,
}

impl ControlBuilder {
    pub(crate) fn new<N>(name: N, architecture: Architecture) -> Self
    where
        N: fmt::Display,
    {
        Self {
            name: name.to_string(),
            architecture,
            version: None,
            description: None,
            depends: Vec::new(),
        }
    }

    fn control(&self, files: &[FileBuilder]) -> Result<Vec<u8>> {
        let mut o = Vec::new();

        writeln!(o, "Package: {}", self.name)?;
        writeln!(o, "Architecture: {}", self.architecture)?;

        if let Some(version) = &self.version {
            writeln!(o, "Version: {version}")?;
        }

        if let Some(description) = &self.description {
            writeln!(o, "Description: {description}")?;
        } else {
            writeln!(o, "Description: ")?;
        }

        let installed_size = files.iter().map(|f| f.contents.len()).sum::<usize>();
        writeln!(o, "Installed-Size: {installed_size}")?;

        match &self.depends[..] {
            [] => {}
            [depend] => writeln!(o, "Depends: {depend}")?,
            depends => {
                writeln!(o, "Depends:")?;

                let mut it = depends.iter().peekable();

                while let Some(depend) = it.next() {
                    write!(o, " {depend}")?;

                    if it.peek().is_some() {
                        writeln!(o, ",")?;
                    }
                }
            }
        }

        Ok(o)
    }

    fn md5sums(&self, files: &[FileBuilder]) -> Result<Vec<u8>> {
        let mut o = Vec::new();

        for file in files {
            let checksum = md5::compute(&file.contents);
            writeln!(o, "{checksum:x} {}", file.path)?;
        }

        Ok(o)
    }

    fn conffiles(&self) -> Result<Vec<u8>> {
        let o = Vec::new();

        Ok(o)
    }

    fn shlibs(&self) -> Result<Vec<u8>> {
        let o = Vec::new();

        Ok(o)
    }

    fn build(self, files: &[FileBuilder]) -> Result<Vec<u8>> {
        let compression = xz2::write::XzEncoder::new(Vec::new(), 9);
        let mut builder = tar::Builder::new(compression);

        let control = self.control(files)?;

        let mut header = tar::Header::new_gnu();
        header.set_path("control")?;
        header.set_size(control.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(&control))?;

        let md5sums = self.md5sums(files)?;
        let mut header = tar::Header::new_gnu();
        header.set_path("md5sums")?;
        header.set_size(md5sums.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(&md5sums))?;

        let conffiles = self.conffiles()?;
        let mut header = tar::Header::new_gnu();
        header.set_path("conffiles")?;
        header.set_size(conffiles.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(&conffiles))?;

        let shlibs = self.shlibs()?;
        let mut header = tar::Header::new_gnu();
        header.set_path("shlibs")?;
        header.set_size(shlibs.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(&shlibs))?;

        Ok(builder.into_inner()?.finish()?)
    }
}

pub(crate) struct DataBuilder {
    files: Vec<FileBuilder>,
}

impl DataBuilder {
    pub(crate) fn new() -> Self {
        Self { files: Vec::new() }
    }

    fn build(self) -> Result<Vec<u8>> {
        let compression = xz2::write::XzEncoder::new(Vec::new(), 9);
        let mut builder = tar::Builder::new(compression);

        for file in &self.files {
            let mut header = tar::Header::new_gnu();
            header
                .set_path(file.path.as_str())
                .with_context(|| anyhow!("Setting path {}", &file.path))?;

            header.set_size(file.contents.len() as u64);
            header.set_mode(file.mode);
            header.set_mtime(file.mtime);
            header.set_uid(file.uid);
            header.set_gid(file.gid);
            header.set_cksum();
            builder.append(&header, Cursor::new(&file.contents))?;
        }

        Ok(builder.into_inner()?.finish()?)
    }
}

pub(crate) struct FileBuilder {
    path: RelativePathBuf,
    mode: u32,
    mtime: u64,
    contents: Vec<u8>,
    uid: u64,
    gid: u64,
}

impl FileBuilder {
    pub(crate) fn new<P>(path: P) -> Self
    where
        P: AsRef<RelativePath>,
    {
        Self {
            path: path.as_ref().to_owned(),
            mode: 0o644,
            mtime: 0,
            contents: Vec::new(),
            uid: 0,
            gid: 0,
        }
    }

    /// Set the mode of the file.
    pub(crate) fn mode(&mut self, mode: u32) -> &mut Self {
        self.mode = mode;
        self
    }

    /// Set the modification time of the file.
    pub(crate) fn mtime(&mut self, time: SystemTime) -> Result<&mut Self> {
        let mtime = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .context("Converting system time")?
            .as_secs();
        self.mtime = mtime;
        Ok(self)
    }

    /// Set the contents of the file.
    pub(crate) fn contents(&mut self, contents: Vec<u8>) -> &mut Self {
        self.contents = contents;
        self
    }
}

/// The builder of a dependency.
pub struct DependsBuilder {
    name: String,
    version: Option<String>,
}

impl DependsBuilder {
    /// Create a new dependency builder.
    pub(crate) fn new<N>(name: N) -> Self
    where
        N: fmt::Display,
    {
        Self {
            name: name.to_string(),
            version: None,
        }
    }

    /// Set the version of the dependency.
    pub(crate) fn version<V>(&mut self, version: V) -> &mut Self
    where
        V: fmt::Display,
    {
        self.version = Some(version.to_string());
        self
    }
}

impl fmt::Display for DependsBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;

        if let Some(version) = &self.version {
            write!(f, " ({version})")?;
        }

        Ok(())
    }
}
