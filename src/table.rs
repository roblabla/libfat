//! FATs managment.

use super::filesystem::FatFileSystem;
use super::utils::FileSystemIterator;
use super::Cluster;
use super::FatError;
use super::FatFileSystemResult;
use super::FatFsType;
use storage_device::StorageDevice;

#[derive(Debug, Copy, Clone, PartialEq)]
/// Represent a cluster chain value.
pub enum FatValue {
    /// Represent a free cluster.
    Free,

    /// Represent a used cluster.
    Data(u32),

    /// Represent a corrupted cluster (bad sectors)
    Bad,

    /// Represent the end of a cluster chain.
    EndOfChain,
}

/// Util iterator used to simplify iteration over cluster.
pub struct FatClusterIter {
    /// The last cluster returned.
    current_cluster: Option<Cluster>,

    /// The last FatValue used.
    last_fat: Option<FatValue>,
}

impl FatClusterIter {
    /// Create a new Cluster iterator starting at ``cluster``.
    pub fn new<S: StorageDevice>(fs: &FatFileSystem<S>, cluster: Cluster) -> FatClusterIter {
        let fat_value = FatValue::get(fs, cluster).ok();
        FatClusterIter {
            current_cluster: Some(cluster),
            last_fat: fat_value,
        }
    }
}

impl<S: StorageDevice> FileSystemIterator<S> for FatClusterIter {
    type Item = Cluster;
    fn next(&mut self, filesystem: &FatFileSystem<S>) -> Option<Cluster> {
        let res = self.current_cluster?;

        match self.last_fat {
            Some(FatValue::Data(data)) => {
                self.current_cluster = Some(Cluster(data));
                self.last_fat = FatValue::get(filesystem, self.current_cluster?).ok();
            }
            _ => self.current_cluster = None,
        };

        Some(res)
    }
}

impl FatValue {
    /// Create a ``FatValue`` from a raw FAT32 value.
    fn from_fat32_value(val: u32) -> Self {
        match val {
            0 => FatValue::Free,
            0x0FFF_FFF7 => FatValue::Bad,
            0x0FFF_FFF8..=0x0FFF_FFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        }
    }

    /// Convert a ```FatValue``` to a raw FAT32 value.
    fn to_fat32_value(self) -> u32 {
        match self {
            FatValue::Free => 0,
            FatValue::Bad => 0x0FFF_FFF7,
            FatValue::EndOfChain => 0x0FFF_FFFF,
            FatValue::Data(n) => n,
        }
    }

    /// Create a ``FatValue`` from a raw FAT16 value.
    fn from_fat16_value(val: u16) -> Self {
        match val {
            0 => FatValue::Free,
            0xFFF7 => FatValue::Bad,
            0xFFF8..=0xFFFF => FatValue::EndOfChain,
            n => FatValue::Data(u32::from(n)),
        }
    }

    /// Convert a ```FatValue``` to a raw FAT16 value.
    fn to_fat16_value(self) -> u16 {
        match self {
            FatValue::Free => 0,
            FatValue::Bad => 0xFFF7,
            FatValue::EndOfChain => 0xFFFF,
            FatValue::Data(n) => n as u16,
        }
    }

    /// Create a ``FatValue`` from a raw FAT12 value.
    fn from_fat12_value(val: u16) -> Self {
        match val {
            0 => FatValue::Free,
            0xFF7 => FatValue::Bad,
            0xFF8..=0xFFF => FatValue::EndOfChain,
            n => FatValue::Data(u32::from(n)),
        }
    }

    /// Convert a ```FatValue``` to a raw FAT12 value.
    fn to_fat12_value(self) -> u16 {
        match self {
            FatValue::Free => 0,
            FatValue::Bad => 0xFF7,
            FatValue::EndOfChain => 0xFFF,
            FatValue::Data(n) => n as u16,
        }
    }

    /// Create a ```FatValue``` from a raw cluster.
    /// Used internally in get and raw_put.
    fn from_cluster<S: StorageDevice>(
        fs: &FatFileSystem<S>,
        cluster: Cluster,
        fat_index: u32,
    ) -> FatFileSystemResult<(Self, u64)> {
        let fat_offset = cluster.to_fat_offset(fs.boot_record.fat_type);
        let cluster_storage_offset = cluster.to_fat_bytes_offset(fs)
            + u64::from(fat_index * fs.boot_record.fat_size())
                * u64::from(fs.boot_record.bytes_per_block());

        let cluster_offset = u64::from(fat_offset % u32::from(fs.boot_record.bytes_per_block()));
        let partition_storage_offset = fs.partition_start + cluster_storage_offset + cluster_offset;

        match fs.boot_record.fat_type {
            FatFsType::Fat32 => {
                let mut data = [0x0u8; 4];
                fs.storage_device
                    .lock()
                    .read(partition_storage_offset, &mut data)
                    .or(Err(FatError::ReadFailed))?;

                Ok((
                    Self::from_fat32_value(u32::from_le_bytes(data) & 0x0FFF_FFFF),
                    cluster_storage_offset,
                ))
            }
            FatFsType::Fat16 => {
                let mut data = [0x0u8; 2];
                fs.storage_device
                    .lock()
                    .read(partition_storage_offset, &mut data)
                    .or(Err(FatError::ReadFailed))?;

                Ok((
                    Self::from_fat16_value(u16::from_le_bytes(data)),
                    cluster_storage_offset,
                ))
            }
            FatFsType::Fat12 => {
                let mut data = [0x0u8; 2];
                fs.storage_device
                    .lock()
                    .read(partition_storage_offset, &mut data)
                    .or(Err(FatError::ReadFailed))?;

                let mut value = u16::from_le_bytes(data);

                value = if (cluster.0 & 1) == 1 {
                    value >> 4
                } else {
                    value & 0x0FFF
                };

                Ok((Self::from_fat12_value(value), cluster_storage_offset))
            }
        }
    }

    /// Get the ```FatValue``` of a given cluster.
    pub fn get<S: StorageDevice>(
        fs: &FatFileSystem<S>,
        cluster: Cluster,
    ) -> FatFileSystemResult<FatValue> {
        Ok(FatValue::from_cluster(fs, cluster, 0)?.0)
    }

    /// Write the given ``FatValue``at a given ``Cluster`` in one FAT.
    fn raw_put<S: StorageDevice>(
        fs: &FatFileSystem<S>,
        cluster: Cluster,
        value: FatValue,
        fat_index: u32,
    ) -> FatFileSystemResult<()> {
        let (res, cluster_storage_offset) = FatValue::from_cluster(fs, cluster, fat_index)?;

        let fat_offset = cluster.to_fat_offset(fs.boot_record.fat_type);
        let cluster_offset = u64::from(fat_offset % u32::from(fs.boot_record.bytes_per_block()));
        let partition_storage_offset = fs.partition_start + cluster_storage_offset + cluster_offset;

        // no write needed
        if res == value {
            return Ok(());
        }

        match fs.boot_record.fat_type {
            FatFsType::Fat32 => {
                let value = value.to_fat32_value() & 0x0FFF_FFFF;

                fs.storage_device
                    .lock()
                    .write(partition_storage_offset, &value.to_le_bytes())
                    .or(Err(FatError::WriteFailed))?;
            }
            FatFsType::Fat16 => {
                fs.storage_device
                    .lock()
                    .write(
                        partition_storage_offset,
                        &value.to_fat16_value().to_le_bytes(),
                    )
                    .or(Err(FatError::WriteFailed))?;
            }
            FatFsType::Fat12 => {
                // Welcome to the IBM world
                let mut data = [0x0u8; 2];

                fs.storage_device
                    .lock()
                    .read(partition_storage_offset, &mut data)
                    .or(Err(FatError::ReadFailed))?;

                let value = value.to_fat12_value();

                if (cluster.0 & 1) == 1 {
                    data[0] = (data[0] & 0x0F) | (value << 4) as u8;
                    data[1] = (value >> 4) as u8;
                } else {
                    data[0] = value as u8;
                    data[1] = (data[0] & 0xF0) | ((value >> 8) & 0x0F) as u8;
                }

                fs.storage_device
                    .lock()
                    .write(partition_storage_offset, &data)
                    .or(Err(FatError::WriteFailed))?;
            }
        }

        Ok(())
    }

    /// Write the given ``FatValue``at a given ``Cluster`` in all FATs.
    pub fn put<S: StorageDevice>(
        fs: &FatFileSystem<S>,
        cluster: Cluster,
        value: FatValue,
    ) -> FatFileSystemResult<()> {
        for fat_index in 0..u32::from(fs.boot_record.fats_count()) {
            Self::raw_put(fs, cluster, value, fat_index)?;
        }
        Ok(())
    }

    /// Initialize clean FATs.
    pub(crate) fn initialize<S: StorageDevice>(fs: &FatFileSystem<S>) -> FatFileSystemResult<()> {
        for i in 0..fs.boot_record.cluster_count {
            Self::put(fs, Cluster(i), FatValue::Free)?;
        }
        Ok(())
    }
}

/// Get the last cluster of a cluster chain.
pub fn get_last_cluster<S: StorageDevice>(
    fs: &FatFileSystem<S>,
    cluster: Cluster,
) -> FatFileSystemResult<Cluster> {
    Ok(get_last_and_previous_cluster(fs, cluster)?.0)
}

/// Get the last cluster and prevous cluster of a cluster chain.
pub fn get_last_and_previous_cluster<S: StorageDevice>(
    fs: &FatFileSystem<S>,
    cluster: Cluster,
) -> FatFileSystemResult<(Cluster, Option<Cluster>)> {
    let mut previous_cluster = None;
    let mut current_cluster = cluster;

    while let FatValue::Data(val) = FatValue::get(fs, current_cluster)? {
        previous_cluster = Some(current_cluster);
        current_cluster = Cluster(val);
    }

    Ok((current_cluster, previous_cluster))
}

/// Compute the whole cluster count of a given FileSystem.
pub fn get_free_cluster_count<S: StorageDevice>(fs: &FatFileSystem<S>) -> FatFileSystemResult<u32> {
    let mut current_cluster = Cluster(2);

    let mut res = 0;

    while current_cluster.0 < fs.boot_record.cluster_count {
        if let FatValue::Free = FatValue::get(fs, current_cluster)? {
            res += 1;
        }

        current_cluster = Cluster(current_cluster.0 + 1);
    }

    Ok(res)
}
