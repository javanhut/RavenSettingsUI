//! Disks and filesystems, from `lsblk` and `df`.

use anyhow::Result;
use serde::Deserialize;

use crate::util::run;

#[derive(Debug, Clone, Deserialize)]
pub struct BlockDevice {
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub mountpoint: Option<String>,
    #[serde(default)]
    pub fstype: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub children: Vec<BlockDevice>,
}

#[derive(Debug, Deserialize)]
struct Lsblk {
    blockdevices: Vec<BlockDevice>,
}

#[derive(Debug, Clone)]
pub struct Filesystem {
    pub target: String,
    pub source: String,
    pub size: u64,
    pub used: u64,
    pub avail: u64,
}

pub fn devices() -> Result<Vec<BlockDevice>> {
    let json = run(
        "lsblk",
        &[
            "-J",
            "-b",
            "-o",
            "NAME,SIZE,TYPE,MOUNTPOINT,FSTYPE,LABEL,MODEL",
        ],
    )?;
    let parsed: Lsblk = serde_json::from_str(&json)?;
    Ok(parsed.blockdevices)
}

pub fn filesystems() -> Result<Vec<Filesystem>> {
    let text = run(
        "df",
        &[
            "-B1",
            "--output=target,source,size,used,avail",
            "-x",
            "tmpfs",
            "-x",
            "devtmpfs",
            "-x",
            "efivarfs",
        ],
    )?;
    Ok(parse_df(&text))
}

pub fn parse_df(text: &str) -> Vec<Filesystem> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 5 {
                return None;
            }
            Some(Filesystem {
                target: f[0].into(),
                source: f[1].into(),
                size: f[2].parse().ok()?,
                used: f[3].parse().ok()?,
                avail: f[4].parse().ok()?,
            })
        })
        .filter(|fs| fs.size > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_parses() {
        let t = "Mounted on Filesystem 1B-blocks Used Available\n/ /dev/nvme0n1p3 100 40 60\n/boot/efi /dev/nvme0n1p1 10 1 9\n";
        let v = parse_df(t);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].target, "/");
        assert_eq!(v[0].avail, 60);
    }
}
