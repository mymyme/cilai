//! 备份符号链接本身，不读取或复制链接目标。
use std::{io, path::Path};

/// 将符号链接的目标原样记录到备份目录。
/// # Errors
/// 源路径不是符号链接、目标不可创建或系统权限不足时返回错误。
pub fn copy_symbolic_link(source: &Path, destination: &Path) -> io::Result<()> {
    let target = std::fs::read_link(source)?;
    create_link(source, destination, &target)
}

/// Replace a staged link without following its target.
/// # Errors
/// Returns an error if the staged link cannot be replaced on this platform.
pub fn replace_symbolic_link_target(link: &Path, target: &Path) -> io::Result<()> {
    let temporary = link.with_extension("cilai-link-migration");
    create_link(link, &temporary, target)?;
    #[cfg(target_os = "macos")]
    std::fs::remove_file(link)?;
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::FileTypeExt;
        if std::fs::symlink_metadata(link)?
            .file_type()
            .is_symlink_dir()
        {
            std::fs::remove_dir(link)?;
        } else {
            std::fs::remove_file(link)?;
        }
    }
    std::fs::rename(temporary, link)
}

fn create_link(source: &Path, destination: &Path, target: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = source;
        std::os::unix::fs::symlink(target, destination)
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::FileTypeExt;
        if std::fs::symlink_metadata(source)?
            .file_type()
            .is_symlink_dir()
        {
            std::os::windows::fs::symlink_dir(target, destination)
        } else {
            std::os::windows::fs::symlink_file(target, destination)
        }
    }
}
