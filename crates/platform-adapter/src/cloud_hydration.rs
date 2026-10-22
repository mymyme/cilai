//! 用户明确读取时请求系统下载占位正文；等待由调用方调度，不阻塞业务状态锁。
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::SystemTime,
};

static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);

#[must_use]
pub fn same_modified_time(before: Option<SystemTime>, after: Option<SystemTime>) -> bool {
    platform::same_modified_time(before, after)
}

pub struct HydrationRequest {
    result: mpsc::Receiver<io::Result<()>>,
    accepted: bool,
}

impl HydrationRequest {
    /// # Errors
    /// 系统拒绝下载或下载工作线程失败时返回错误。
    pub fn poll(&mut self) -> io::Result<()> {
        if self.accepted {
            return Ok(());
        }
        match self.result.try_recv() {
            Ok(result) => {
                result?;
                self.accepted = true;
                Ok(())
            }
            Err(mpsc::TryRecvError::Empty) => Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(io::Error::other("hydration_worker_stopped"))
            }
        }
    }
}

/// # Errors
/// 请求数超限、系统不支持或者不能启动工作线程时拒绝，不强读占位文件。
pub fn request_download(path: &Path) -> io::Result<HydrationRequest> {
    ACTIVE_REQUESTS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 4).then_some(n + 1)
        })
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "hydration_busy"))?;
    let (tx, rx) = mpsc::channel();
    let path = PathBuf::from(path);
    let spawned = std::thread::Builder::new()
        .name("cloud-file-hydration".into())
        .spawn(move || {
            struct Active;
            impl Drop for Active {
                fn drop(&mut self) {
                    ACTIVE_REQUESTS.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _active = Active;
            let _ = tx.send(platform::request(&path));
        });
    if let Err(error) = spawned {
        ACTIVE_REQUESTS.fetch_sub(1, Ordering::AcqRel);
        return Err(error);
    }
    Ok(HydrationRequest {
        result: rx,
        accepted: false,
    })
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    use std::{io, path::Path};
    pub(super) fn same_modified_time(
        before: Option<std::time::SystemTime>,
        after: Option<std::time::SystemTime>,
    ) -> bool {
        let interval = |time: std::time::SystemTime| {
            time.duration_since(std::time::UNIX_EPOCH)
                .map_or_else(
                    |error| -error.duration().as_secs_f64(),
                    |value| value.as_secs_f64(),
                )
                .to_bits()
        };
        before.map(interval) == after.map(interval)
    }
    pub(super) fn request(path: &Path) -> io::Result<()> {
        let value = path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path_encoding"))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(value));
        let manager = NSFileManager::defaultManager();
        if !manager.isUbiquitousItemAtURL(&url) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "provider_unsupported",
            ));
        }
        manager
            .startDownloadingUbiquitousItemAtURL_error(&url)
            .map_err(|_| io::Error::other("provider_download_rejected"))
    }
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod platform {
    use std::{
        fs::OpenOptions,
        io,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::Path,
    };
    use windows_sys::Win32::Storage::{
        CloudFilters::{CF_HYDRATE_FLAG_NONE, CfHydratePlaceholder},
        FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
    };
    pub(super) fn same_modified_time(
        before: Option<std::time::SystemTime>,
        after: Option<std::time::SystemTime>,
    ) -> bool {
        before == after
    }
    pub(super) fn request(path: &Path) -> io::Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        // SAFETY: file拥有有效句柄直到同步调用返回；没有跨线程借用OVERLAPPED或其他指针。
        let result = unsafe {
            CfHydratePlaceholder(
                file.as_raw_handle(),
                0,
                -1,
                CF_HYDRATE_FLAG_NONE,
                std::ptr::null_mut(),
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "provider_download_rejected:{result:#x}"
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn windows_hydration_rejects_non_provider_files_and_preserves_their_contents() {
            let root = std::env::temp_dir().join(format!(
                "cilai-hydration 中文 {}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("普通 文件.txt");
            std::fs::write(&path, b"keep original content").unwrap();
            let error = request(&path).unwrap_err();
            assert!(error.to_string().starts_with("provider_download_rejected:"));
            assert_eq!(std::fs::read(&path).unwrap(), b"keep original content");
            assert_eq!(
                request(&root.join("missing.txt")).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{io, path::Path};
    pub(super) fn same_modified_time(
        before: Option<std::time::SystemTime>,
        after: Option<std::time::SystemTime>,
    ) -> bool {
        before == after
    }
    pub(super) fn request(_path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "provider_unsupported",
        ))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::same_modified_time;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn hydration_accepts_nsdate_round_trip_but_rejects_a_real_time_change() {
        let before = UNIX_EPOCH + Duration::new(1_791_999_211, 839_228_555);
        let after = UNIX_EPOCH + Duration::new(1_791_999_211, 839_228_630);
        assert!(same_modified_time(Some(before), Some(after)));
        assert!(!same_modified_time(
            Some(before),
            Some(after + Duration::from_micros(1))
        ));
        assert!(!same_modified_time(Some(before), None));
        assert!(!same_modified_time(None, Some(after)));
    }

    #[test]
    fn hydration_time_comparison_keeps_pre_epoch_dates_distinct() {
        let before = UNIX_EPOCH - Duration::from_secs(10);
        assert!(same_modified_time(Some(before), Some(before)));
        assert!(!same_modified_time(
            Some(before),
            Some(before - Duration::from_secs(1))
        ));
    }
}
