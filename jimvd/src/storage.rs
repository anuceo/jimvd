use anyhow::Result;

#[allow(dead_code)]   // will be removed once we implement I/O
pub struct VirtualDisk {
    base_path: String,
}

#[allow(dead_code)]
impl VirtualDisk {
    pub fn new(path: &str) -> Self {
        VirtualDisk {
            base_path: path.to_string(),
        }
    }

    pub fn save_blocks(
        &self,
        _db_id: i32,
        _version: &str,
        _blocks: &[crate::types::Factor],
    ) -> Result<()> {
        // TODO: write to JSON file
        Ok(())
    }

    pub fn load_blocks(
        &self,
        _db_id: i32,
        _version: &str,
    ) -> Result<Vec<crate::types::Factor>> {
        Ok(vec![])
    }
}