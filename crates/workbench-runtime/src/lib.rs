//! 词来工作台的桌面端业务逻辑。
//!
//! 这些模块此前是宿主壳可执行 crate 的内部模块。它们不依赖界面框架，却也因此无法被
//! 另一个壳引用——可执行 crate 的内部模块对外不可见。迁移期靠源码路径穿透引用加上
//! 局部复制来绕开，代价是依赖表各维护一份、复制的部分会静默漂移。提取为库之后两个壳
//! 引同一份实现，这两笔代价都消失。
//!
//! 边界：本 crate 只放不依赖界面框架的部分。命令标注、窗口与视图操作属于各自的壳，
//! 留在壳内，不随本次提取移动。

pub mod ai_runtime;
pub mod app_bridge;
mod application_result;
mod application_tool_adapter;
pub mod async_support;
pub mod codex_harness;
mod codex_threads;
pub mod codex_upgrade;
mod context_import;
mod conversation_kernel;
pub mod desktop_rpc;
mod harness_pool;
pub mod mcp_connections;
pub mod memory;
pub mod model_connections;
mod model_session_proxy;
mod persistent_threads;
mod personal_exec;
pub mod personal_page_ai;
pub mod personal_runtime;
mod report_result;
mod report_workspace;
mod request_timing;
pub mod supervisor;
pub mod windows_sandbox;
mod work_classification;
mod workbench_result;
pub mod workspace;
