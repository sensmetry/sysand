// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Read;

/// A file that is guaranteed to exist as long as the lifetime.
/// Intended to be used with temporary files that are automatically
/// deleted; in this case, the lifetime `'a` is the lifetime of the
/// temporary file.
pub struct FileWithLifetime<'a> {
    internal: std::fs::File,
    phantom: std::marker::PhantomData<&'a ()>,
}

impl FileWithLifetime<'_> {
    pub fn new(file: std::fs::File) -> Self {
        FileWithLifetime {
            internal: file,
            phantom: std::marker::PhantomData,
        }
    }
}

impl Read for FileWithLifetime<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.internal.read(buf)
    }
}
