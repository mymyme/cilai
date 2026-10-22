//! 本机资料写者的登记启动门；权限和工作区校验仍由调用方完成。
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;

pub const COMMAND: &str = "personal-writer-launch-gate";
pub const PERMIT: &[u8] = b"CILAI_PERSONAL_WRITER_PERMIT_V1\n";

/// 保留已校验命令的环境与目录，先运行受信宿主启动门。
/// # Errors
/// 目标或宿主不是绝对路径、工作目录缺失时拒绝。
pub fn wrap_command(original: &Command, helper: &Path) -> io::Result<Command> {
    let program = Path::new(original.get_program());
    if !helper.is_absolute() || !program.is_absolute() || original.get_current_dir().is_none() {
        return Err(io::Error::other("personal_writer_launch_scope_invalid"));
    }
    crate::executable_search::validate_executable_file(helper)?;
    crate::executable_search::validate_executable_file(program)?;
    let mut command = Command::new(helper);
    command
        .arg(COMMAND)
        .arg("--")
        .arg(program)
        .args(original.get_args());
    command.current_dir(
        original
            .get_current_dir()
            .ok_or_else(|| io::Error::other("personal_writer_launch_scope_invalid"))?,
    );
    command.env_clear();
    for (key, value) in original.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    Ok(command)
}

fn read_permit(input: &mut impl Read) -> io::Result<bool> {
    for expected in PERMIT {
        let mut byte = [0];
        match input.read_exact(&mut byte) {
            Ok(()) if byte[0] == *expected => (),
            Ok(()) => return Err(io::Error::other("personal_writer_permit_invalid")),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

/// 只消费许可行，后续 stdin 原封不动交给原隔离命令。
/// # Errors
/// 参数、许可或受监督目标无法执行时返回错误。
pub fn run(arguments: &[OsString]) -> io::Result<i32> {
    let Some((separator, rest)) = arguments.split_first() else {
        return Err(io::Error::other("personal_writer_launch_arguments_invalid"));
    };
    if separator != "--" || rest.is_empty() || !Path::new(&rest[0]).is_absolute() {
        return Err(io::Error::other("personal_writer_launch_arguments_invalid"));
    }
    crate::executable_search::validate_executable_file(Path::new(&rest[0]))?;
    let mut input = crate::runtime_launch_gate::unbuffered_stdin()?;
    if !read_permit(&mut input)? {
        return Ok(0);
    }
    let mut command = Command::new(&rest[0]);
    command.args(&rest[1..]);
    crate::runtime_launch_gate::execute(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permit_does_not_consume_the_first_mcp_message() {
        let mut bytes = PERMIT.to_vec();
        bytes.extend_from_slice(b"{\"jsonrpc\":\"2.0\"}\n");
        let mut input = io::Cursor::new(bytes);
        assert!(read_permit(&mut input).unwrap());
        let mut rest = String::new();
        input.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "{\"jsonrpc\":\"2.0\"}\n");
        for end in 0..PERMIT.len() {
            assert!(!read_permit(&mut io::Cursor::new(&PERMIT[..end])).unwrap());
        }
        assert!(read_permit(&mut io::Cursor::new(b"wrong\n")).is_err());
    }
}
