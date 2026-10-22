//! 异步执行支撑。
//!
//! 少数几处业务逻辑需要在同步上下文中等待一个 future，或把阻塞调用移出异步线程。原先
//! 这两件事由界面框架的全局运行时提供，业务库不该为此依赖任一个壳，因此自建一个。
//!
//! 运行时是进程级单例：这几处调用出现在回调与目录读取里，为每次调用新建运行时既浪费
//! 线程也会让阻塞任务各自为政。

/// 在同步上下文中等待一个 future 完成。
///
/// 只用于确实身处同步上下文的位置，例如交给对话运行时的回调。在异步上下文中调用会
/// 触发运行时的重入检查而 panic，这是应有的行为——那种位置本就该直接 await。
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    runtime().block_on(future)
}

/// 把阻塞调用移出异步线程。
pub fn spawn_blocking<F, T>(task: F) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    runtime().spawn_blocking(task)
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static INSTANCE: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("无法创建业务库的异步运行时")
    })
}
