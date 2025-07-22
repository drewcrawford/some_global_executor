use r#continue::continuation;
use some_executor::task::{Configuration, Task};
use some_executor::SomeExecutor;
use some_executor::observer::Observer;

//for the time being, wasm_thread only works in browser
//see https://github.com/rustwasm/wasm-bindgen/issues/4534,
//though we also need wasm_thread support.
#[cfg(target_arch="wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[test_executors::async_test] async fn test_static_inline() {
    let (c,f) = continuation();
    let mut global_executor = some_global_executor::Executor::new("test_static_inline".to_string(), 100);
    let systems_task = Task::without_notifications("Systems".to_string(), Configuration::default(), async move {
        //need a non-send task to bring them up
        let systems_inner_task = Task::without_notifications("Systems (non-Send)".to_string(), Configuration::default(), async move {
            c.send(());
        });
        systems_inner_task.spawn_static_current();
    });
    global_executor.spawn(systems_task).detach();
    f.await;
}