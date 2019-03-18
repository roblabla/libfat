use arrayvec::ArrayString;

use crate::fat::detail::attribute::Attributes;
use crate::fat::detail::block::{BlockDevice, BlockIndex};
use crate::fat::detail::cluster::Cluster;

use crate::FileSystemError;
use crate::fat::detail::filesystem::FatFileSystem;
use crate::Result as FileSystemResult;

use super::raw_dir_entry::FatDirEntry;


#[derive(Debug, Clone, Copy)]
pub(crate) struct DirectoryEntryRawInfo {
    pub parent_cluster: Cluster,
    pub first_entry_block_index: BlockIndex,
    pub first_entry_offset: u32,
    pub entry_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DirectoryEntry {
    pub start_cluster: Cluster,
    pub(crate) raw_info: Option<DirectoryEntryRawInfo>,
    pub creation_timestamp: u64,
    pub last_access_timestamp: u64,
    pub last_modification_timestamp: u64,
    pub file_size: u32,
    pub file_name: ArrayString<[u8; Self::MAX_FILE_NAME_LEN]>,
    pub attribute: Attributes,
}

impl DirectoryEntryRawInfo {
    pub fn get_dir_entry<T>(&self, fs: &FatFileSystem<T>) -> FileSystemResult<FatDirEntry>
    where
        T: BlockDevice,
    {
        let mut block_iter = super::FatDirEntryIterator::new(
            fs,
            self.parent_cluster,
            self.first_entry_block_index,
            self.first_entry_offset,
        );

        let mut i = 0;
        let mut res = None;

        while i < self.entry_count {
            let result = block_iter.next();
            if let Some(result) = result {
                res = Some(result?);
            } else {
                res = None;
            }
            i += 1;
        }

        if let Some(res) = res {
            Ok(res)
        } else {
            Err(FileSystemError::NotFound)
        }
    }
}

impl DirectoryEntry {
    // entry can at best have 255 chars in UTF-16
    pub const MAX_FILE_NAME_LEN: usize = 256 * 4;
}
