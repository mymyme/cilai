//! 工作区文件与 PDF 预览。
//!
//! 路径校验、文件打开与 PDF 渲染都不依赖界面框架，但原先与命令层混在宿主壳的主模块里，
//! 另一个壳引不到，只能携带一份副本。提取到这里之后副本删除，两个壳引同一份。
//!
//! 现有壳的命令仍内联着各自的实现，退役时随该文件一并删去；在此之前两处并存，改动其一
//! 须同时改另一处。

use base64::Engine as _;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

const MAX_CLIPBOARD_TEXT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CLIPBOARD_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceApplication {
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceClipboardContent {
    pub kind: &'static str,
    pub text: Option<String>,
    pub data_base64: Option<String>,
}

/// # Errors
/// 目录不是绝对路径、不存在或不是文件夹时返回错误。
pub fn validated_directory(local_root: &str) -> Result<PathBuf, String> {
    let path = Path::new(local_root);
    if !path.is_absolute() {
        return Err("workspace_directory_not_absolute".to_owned());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "workspace_directory_unavailable".to_owned())?;
    if !canonical.is_dir() {
        return Err("workspace_directory_not_directory".to_owned());
    }
    Ok(canonical)
}

/// 把工作区相对路径解析为真实路径，越界一律拒绝。
/// # Errors
/// 相对路径越界、文件不存在或解析后超出工作区时返回错误。
pub fn resolved_item(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("workspace_item_path_invalid".to_owned());
    }
    let canonical = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "workspace_item_unavailable".to_owned())?;
    if !canonical.starts_with(root) {
        return Err("workspace_item_outside_root".to_owned());
    }
    Ok(canonical)
}

/// 打开方式仅接受三种取值，其余一律拒绝——放行未知取值等于把任意程序名交给系统。
/// # Errors
/// 打开方式不受支持或系统无法打开文件时返回错误。
pub fn open_item(item: &Path, open_mode: &str) -> Result<(), String> {
    use contexthub_platform_adapter::system;
    let outcome = match open_mode {
        "finder" => system::reveal_in_file_manager(item),
        "default" => system::open_with_default_handler(item),
        "vscode" => system::open_with_visual_studio_code(item),
        _ => return Err("workspace_item_open_mode_invalid".to_owned()),
    };
    outcome.map_err(|error| format!("workspace_item_open_failed:{error}"))
}

/// # Errors
/// 系统无法查询该文件的默认打开应用时返回错误。
pub fn default_application(item: &Path) -> Result<Option<WorkspaceApplication>, String> {
    contexthub_platform_adapter::system::default_application_display_name(item)
        .map(|maybe| maybe.map(|display_name| WorkspaceApplication { display_name }))
        .map_err(|error| format!("workspace_application_query_failed:{error}"))
}

/// 将员工在宿主保存对话框中确认的目标写入一份副本。
/// # Errors
/// 来源不是文件、目标不是绝对路径、目标目录无效或复制失败时返回错误。
pub fn copy_item(item: &Path, destination: &str) -> Result<(), String> {
    let source = item
        .canonicalize()
        .map_err(|_| "workspace_item_unavailable".to_owned())?;
    if !source.is_file() {
        return Err("workspace_item_not_file".to_owned());
    }
    let target = Path::new(destination);
    if !target.is_absolute() {
        return Err("workspace_copy_destination_not_absolute".to_owned());
    }
    let parent = target
        .parent()
        .ok_or_else(|| "workspace_copy_destination_invalid".to_owned())?;
    if !parent.is_dir() {
        return Err("workspace_copy_destination_unavailable".to_owned());
    }
    if target.exists() {
        let canonical = target
            .canonicalize()
            .map_err(|_| "workspace_copy_destination_unavailable".to_owned())?;
        if canonical == source {
            return Err("workspace_copy_destination_is_source".to_owned());
        }
        if canonical.is_dir() {
            return Err("workspace_copy_destination_is_directory".to_owned());
        }
    }
    std::fs::copy(source, target)
        .map(|_| ())
        .map_err(|error| format!("workspace_item_copy_failed:{error}"))
}

/// 读取适合写入系统剪贴板的文件内容。图片按原始字节返回，文本必须是有效 UTF-8。
/// # Errors
/// 来源不是文件、内容过大、格式不支持或读取失败时返回错误。
pub fn clipboard_content(item: &Path) -> Result<WorkspaceClipboardContent, String> {
    let metadata = item
        .metadata()
        .map_err(|error| format!("workspace_item_read_failed:{error}"))?;
    if !metadata.is_file() {
        return Err("workspace_item_not_file".to_owned());
    }
    let image = item
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif"
            )
        });
    let limit = if image {
        MAX_CLIPBOARD_IMAGE_BYTES
    } else {
        MAX_CLIPBOARD_TEXT_BYTES
    };
    if metadata.len() > limit {
        return Err("workspace_item_clipboard_too_large".to_owned());
    }
    let bytes =
        std::fs::read(item).map_err(|error| format!("workspace_item_read_failed:{error}"))?;
    if image {
        return Ok(WorkspaceClipboardContent {
            kind: "image",
            text: None,
            data_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        });
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| "workspace_item_clipboard_format_unsupported".to_owned())?;
    Ok(WorkspaceClipboardContent {
        kind: "text",
        text: Some(text),
        data_base64: None,
    })
}

/// PDF 校验与渲染，与现有宿主保持一致。
///
/// 与工作区路径校验同因：位于现有宿主的主模块中，与界面框架代码混杂于同一文件，
/// 无法以源码路径引用。校验规则（扩展名、体积上限、文件头签名）必须与原实现一致，
/// 任一处放宽都会让非 PDF 内容被当作 PDF 读入。渲染本身在独立的辅助进程中完成，
/// 此处只负责调用；正式提取时应与其余工作区能力一并归入工作区文件层。
pub mod pdf {
    use contexthub_platform_adapter::filesystem::{
        is_cloud_placeholder_file, metadata_with_stable_identity,
        open_stable_workspace_file_for_read, opened_file_metadata_with_stable_identity,
    };
    use std::io::Read as _;
    use std::path::Path;

    const MAX_INLINE_BYTES: u64 = 32 * 1024 * 1024;

    /// # Errors
    /// 文件不是有效且允许大小的 PDF 时返回错误。
    pub fn validate_file(item: &Path) -> Result<(), String> {
        validate_extension(item)?;
        let metadata = item
            .metadata()
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        if !metadata.is_file() {
            return Err("workspace_pdf_not_file".to_owned());
        }
        if metadata.len() > MAX_INLINE_BYTES {
            return Err("workspace_pdf_too_large".to_owned());
        }
        let mut signature = Vec::with_capacity(1024);
        std::fs::File::open(item)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?
            .take(1024)
            .read_to_end(&mut signature)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        validate_signature(&signature)
    }

    /// # Errors
    /// 文件身份、内容或路径在读取过程中变化时拒绝返回字节。
    pub fn read_bytes(item: &Path) -> Result<Vec<u8>, String> {
        validate_extension(item)?;
        let before = metadata_with_stable_identity(item)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        if !before.metadata.is_file() {
            return Err("workspace_pdf_not_file".to_owned());
        }
        if is_cloud_placeholder_file(&before.metadata) {
            return Err("workspace_pdf_not_materialized".to_owned());
        }
        if before.metadata.len() > MAX_INLINE_BYTES {
            return Err("workspace_pdf_too_large".to_owned());
        }
        let mut file = open_stable_workspace_file_for_read(item)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        let opened = opened_file_metadata_with_stable_identity(&file)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        if !same_file(&before, &opened) {
            return Err("workspace_pdf_source_changed".to_owned());
        }
        // 读取量受上限约束，预分配也按上限截断，避免按超大文件的声明长度分配内存
        let capacity = usize::try_from(before.metadata.len().min(MAX_INLINE_BYTES + 1))
            .map_err(|_| "workspace_pdf_too_large".to_owned())?;
        let mut bytes = Vec::with_capacity(capacity);
        (&mut file)
            .take(MAX_INLINE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        if bytes.len() as u64 > MAX_INLINE_BYTES {
            return Err("workspace_pdf_too_large".to_owned());
        }
        let opened_after = opened_file_metadata_with_stable_identity(&file)
            .map_err(|error| format!("workspace_pdf_read_failed:{error}"))?;
        let path_after = metadata_with_stable_identity(item)
            .map_err(|_| "workspace_pdf_source_changed".to_owned())?;
        if !same_file(&before, &opened_after)
            || !same_file(&before, &path_after)
            || bytes.len() as u64 != before.metadata.len()
        {
            return Err("workspace_pdf_source_changed".to_owned());
        }
        validate_signature(&bytes[..bytes.len().min(1024)])?;
        Ok(bytes)
    }

    fn same_file(
        first: &contexthub_platform_adapter::filesystem::FileMetadataSnapshot,
        second: &contexthub_platform_adapter::filesystem::FileMetadataSnapshot,
    ) -> bool {
        first.stable_identity == second.stable_identity
            && first.metadata.is_file()
            && second.metadata.is_file()
            && first.metadata.len() == second.metadata.len()
            && matches!(
                (first.metadata.modified(), second.metadata.modified()),
                (Ok(first_time), Ok(second_time)) if first_time == second_time
            )
    }

    fn validate_extension(item: &Path) -> Result<(), String> {
        if item
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            Ok(())
        } else {
            Err("workspace_pdf_format_invalid".to_owned())
        }
    }

    fn validate_signature(contents: &[u8]) -> Result<(), String> {
        let signature_length = contents.len().min(1024);
        if contents[..signature_length]
            .windows(5)
            .any(|window| window == b"%PDF-")
        {
            Ok(())
        } else {
            Err("workspace_pdf_signature_invalid".to_owned())
        }
    }

    /// # Errors
    /// PDF 无法读取、解析或不含可展示页面时返回错误。
    #[cfg(target_os = "macos")]
    pub fn page_count(item: &Path) -> Result<usize, String> {
        let output = run("count", item, &[])?;
        let value = String::from_utf8(output)
            .map_err(|error| format!("workspace_pdf_page_count_invalid:{error}"))?;
        value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|count| *count > 0)
            .ok_or_else(|| "workspace_pdf_has_no_pages".to_owned())
    }

    /// # Errors
    /// 页码无效或 PDF 页面渲染失败时返回错误。
    #[cfg(target_os = "macos")]
    pub fn render_page(item: &Path, page_number: usize, max_width: u32) -> Result<Vec<u8>, String> {
        if page_number == 0 {
            return Err("workspace_pdf_page_out_of_range".to_owned());
        }
        run(
            "render",
            item,
            &[
                page_number.to_string(),
                max_width.clamp(320, 2_400).to_string(),
            ],
        )
    }

    /// 辅助进程取自本进程同目录，因此打包时须与宿主服务一并收集。
    #[cfg(target_os = "macos")]
    fn run(mode: &str, item: &Path, arguments: &[String]) -> Result<Vec<u8>, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("workspace_pdf_renderer_unavailable:{error}"))?;
        let helper = executable
            .parent()
            .ok_or_else(|| "workspace_pdf_renderer_unavailable".to_owned())?
            .join("cilai-pdf-renderer");
        let output = std::process::Command::new(helper)
            .arg(mode)
            .arg(item)
            .args(arguments)
            .output()
            .map_err(|error| format!("workspace_pdf_renderer_unavailable:{error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            let reason = String::from_utf8_lossy(&output.stderr);
            Err(format!("workspace_pdf_renderer_failed:{}", reason.trim()))
        }
    }

    /// # Errors
    /// PDF 无法读取、解析或不含可展示页面时返回错误。
    #[cfg(not(target_os = "macos"))]
    pub fn page_count(_item: &Path) -> Result<usize, String> {
        Err("workspace_pdf_render_unsupported".to_owned())
    }

    /// # Errors
    /// 页码无效或 PDF 页面渲染失败时返回错误。
    #[cfg(not(target_os = "macos"))]
    pub fn render_page(_item: &Path, _page: usize, _max_width: u32) -> Result<Vec<u8>, String> {
        Err("workspace_pdf_render_unsupported".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{clipboard_content, copy_item, pdf::read_bytes};

    #[test]
    fn pdf_bytes_are_read_from_a_validated_stable_file() {
        let directory =
            std::env::temp_dir().join(format!("cilai-pdf-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("合同.pdf");
        let valid = b"%PDF-1.7\nbody";
        std::fs::write(&file, valid).unwrap();
        assert_eq!(read_bytes(&file).unwrap(), valid);
        std::fs::write(&file, b"not a pdf").unwrap();
        assert_eq!(
            read_bytes(&file).unwrap_err(),
            "workspace_pdf_signature_invalid"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn workspace_item_can_be_copied_to_an_employee_selected_destination() {
        let directory =
            std::env::temp_dir().join(format!("cilai-workspace-copy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("产物.txt");
        let destination = directory.join("产物副本.txt");
        std::fs::write(&source, "已完成").unwrap();

        copy_item(&source, destination.to_str().unwrap()).unwrap();

        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "已完成");
        assert_eq!(
            copy_item(&source, source.to_str().unwrap()).unwrap_err(),
            "workspace_copy_destination_is_source"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn clipboard_content_distinguishes_text_and_images() {
        let directory = std::env::temp_dir().join(format!(
            "cilai-workspace-clipboard-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let text = directory.join("说明.md");
        let image = directory.join("生成图片.png");
        std::fs::write(&text, "# 结果").unwrap();
        std::fs::write(&image, [137, 80, 78, 71]).unwrap();

        let text_content = clipboard_content(&text).unwrap();
        assert_eq!(text_content.kind, "text");
        assert_eq!(text_content.text.as_deref(), Some("# 结果"));
        let image_content = clipboard_content(&image).unwrap();
        assert_eq!(image_content.kind, "image");
        assert_eq!(image_content.data_base64.as_deref(), Some("iVBORw=="));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
