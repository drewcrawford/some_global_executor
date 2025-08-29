//! A global executor implementation for the `some_executor` framework.
#![warn(missing_docs)]
//!
//! This crate provides a cross-platform executor that works on both standard platforms
//! and WebAssembly (WASM) targets. It implements the `SomeExecutor` trait and provides
//! thread pool-based task execution with configurable thread counts.
//!
//! # Features
//!
//! - **Cross-platform support**: Works on both native platforms and WASM targets
//! - **Configurable thread pools**: Create executors with custom thread counts
//! - **Task observation**: Monitor task execution through the observer pattern
//! - **Async draining**: Wait for all tasks to complete before shutdown
//! - **Global and thread-local executor support**: Set executors as global or thread-local defaults
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```
//! use some_global_executor::Executor;
//! use some_executor::SomeExecutor;
//! use some_executor::task::{Task, Configuration};
//!
//! // Create an executor with 4 threads
//! let mut executor = Executor::new("my-executor".to_string(), 4);
//!
//! // Spawn a simple task
//! let task = Task::without_notifications(
//!     "example-task".to_string(),
//!     Configuration::default(),
//!     async {
//!         println!("Task executing!");
//!         42
//!     }
//! );
//!
//! let observer = executor.spawn(task);
//! // Task is now running in the background
//! 
//! // Clean up
//! executor.drain();
//! ```
//!
//! ## Using the Default Executor
//!
//! ```
//! use some_global_executor::Executor;
//!
//! // Create an executor with default settings
//! let executor = Executor::new_default();
//! assert_eq!(executor.name(), "default");
//! 
//! // Clean up
//! executor.drain();
//! ```
//!
//! ## Resizing Thread Pool
//!
//! ```
//! use some_global_executor::Executor;
//!
//! let mut executor = Executor::new("resizable".to_string(), 2);
//! 
//! // Resize the thread pool to 8 threads
//! executor.resize(8);
//! 
//! // Clean up
//! executor.drain();
//! ```

use std::any::Any;
use std::convert::Infallible;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use atomic_waker::AtomicWaker;
use logwise::{debuginternal_sync, declare_logging_domain};
use some_executor::DynExecutor;
use some_executor::observer::{Observer, ObserverNotified};
use some_executor::task::{DynSpawnedTask, Task};
use some_executor::SomeExecutor;

/// re-exports the `SomeExecutor` crate for convenience.
pub use some_executor;

declare_logging_domain!();

/// A thread pool-based executor for running asynchronous tasks.
///
/// The `Executor` manages a pool of worker threads that execute submitted tasks.
/// It provides both synchronous and asynchronous draining capabilities, allowing
/// you to wait for all tasks to complete before shutdown.
///
/// # Platform Support
///
/// This executor automatically adapts to the target platform:
/// - On standard platforms, it uses OS threads via `crossbeam-channel`
/// - On WASM targets, it uses web workers for parallelism
///
/// # Examples
///
/// ```
/// use some_global_executor::Executor;
///
/// // Create an executor named "worker-pool" with 4 threads
/// let executor = Executor::new("worker-pool".to_string(), 4);
///
/// // Get the executor's name
/// assert_eq!(executor.name(), "worker-pool");
///
/// // Clean up when done
/// executor.drain();
/// ```
#[derive(Debug,PartialEq, Eq, Hash,Clone)]
pub struct Executor {
    imp: sys::Executor,
    name: String,
}

impl Executor {
    /// Creates a new executor with the specified name and thread count.
    ///
    /// # Arguments
    ///
    /// * `name` - A descriptive name for the executor, used for logging and debugging
    /// * `threads` - The number of worker threads to create in the thread pool
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// // Create an executor with 8 worker threads
    /// let executor = Executor::new("high-performance".to_string(), 8);
    /// 
    /// // Clean up
    /// executor.drain();
    /// ```
    ///
    /// # Logging
    ///
    /// This method logs the creation of the executor using the logwise framework,
    /// including the name and thread count for debugging purposes.
    pub fn new(name: String, threads: usize) -> Self {
        logwise::info_sync!("Creating executor with name {name} and {threads} threads", name=logwise::privacy::LogIt(&name), threads=logwise::privacy::LogIt(threads));
        let name_clone = name.clone();
        let inner = sys::Executor::new(name, threads);
        Executor {
            imp: inner,
            name: name_clone,
        }
    }

    /// Returns the name of this executor.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// let executor = Executor::new("my-executor".to_string(), 2);
    /// assert_eq!(executor.name(), "my-executor");
    /// executor.drain();
    /// ```
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Resizes the thread pool to the specified number of threads.
    ///
    /// This method adjusts the number of worker threads in the executor's thread pool.
    /// The exact behavior depends on the underlying platform implementation.
    ///
    /// # Arguments
    ///
    /// * `threads` - The new number of worker threads
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// let mut executor = Executor::new("dynamic".to_string(), 2);
    /// 
    /// // Increase thread count for heavy workload
    /// executor.resize(8);
    /// 
    /// // Later, reduce thread count
    /// executor.resize(4);
    /// 
    /// executor.drain();
    /// ```
    pub fn resize(&mut self, threads: usize) {
        self.imp.resize(threads);
    }
}

/// A future that completes when all tasks in an executor have finished.
///
/// `ExecutorDrain` is returned by [`Executor::drain_async()`] and implements
/// `Future<Output = ()>`. It polls the executor's internal state to determine
/// when all tasks have completed execution.
///
/// # Examples
///
/// ```
/// # use futures::executor::block_on;
/// use some_global_executor::Executor;
/// use some_executor::SomeExecutor;
/// use some_executor::task::{Task, Configuration};
///
/// # block_on(async {
/// let mut executor = Executor::new("async-drain".to_string(), 2);
///
/// // Spawn some work
/// let task = Task::without_notifications(
///     "work".to_string(),
///     Configuration::default(),
///     async {
///         // Simulate some work
///         42
///     }
/// );
/// executor.spawn(task);
///
/// // Wait for all tasks to complete
/// executor.drain_async().await;
/// # });
/// ```
pub struct ExecutorDrain {
    executor: Executor,
}

impl Future for ExecutorDrain {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        //need to register prior to check
        let drain_notify = self.executor.imp.drain_notify();
        drain_notify.waker.register(cx.waker());
        let running_tasks = drain_notify.running_tasks.load(std::sync::atomic::Ordering::Relaxed);
        debuginternal_sync!("ExecutorDrain::poll running_tasks={running_tasks}",running_tasks=(running_tasks as u64));
        if running_tasks == 0 {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    }
}

#[derive(Debug)]
struct DrainNotify {
    running_tasks: AtomicUsize,
    waker: AtomicWaker
}

impl DrainNotify {
    fn new() -> Self {
        Self {
            running_tasks: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        }
    }
}

mod waker;
mod sys;







#[derive(Debug)]
struct SpawnedTask {
    imp: sys::SpawnedTask,
}


impl SpawnedTask {
    fn new(task: Box<dyn DynSpawnedTask<Infallible>>) -> Self {
        Self {
            imp: sys::SpawnedTask::new(task),
        }
    }
    
}


impl Executor {


    /// Creates a new executor with default settings.
    ///
    /// This creates an executor named "default" with a platform-appropriate
    /// number of threads (determined by the underlying system implementation).
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// let executor = Executor::new_default();
    /// assert_eq!(executor.name(), "default");
    /// executor.drain();
    /// ```
    pub fn new_default() -> Self {
        Self::new("default".to_string(), sys::default_threadpool_size())
    }




    /// Synchronously waits for all tasks in the executor to complete.
    ///
    /// This method blocks the current thread until all spawned tasks have
    /// finished execution. It uses a busy-wait loop with thread yielding
    /// to check for task completion.
    ///
    /// # Performance
    ///
    /// This method logs performance warnings if the drain operation takes
    /// an unexpectedly long time, which may indicate stuck or long-running tasks.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    /// use some_executor::SomeExecutor;
    /// use some_executor::task::{Task, Configuration};
    ///
    /// let mut executor = Executor::new("sync-drain".to_string(), 2);
    ///
    /// // Spawn a task
    /// let task = Task::without_notifications(
    ///     "quick-task".to_string(),
    ///     Configuration::default(),
    ///     async { 1 + 1 }
    /// );
    /// executor.spawn(task);
    ///
    /// // Block until all tasks complete
    /// executor.drain();
    /// ```
    pub fn drain(self) {
        let _interval = logwise::perfwarn_begin!("Executor::drain busyloop");
        loop {
            let running_tasks = self.imp.drain_notify().running_tasks.load(std::sync::atomic::Ordering::Relaxed);
            if running_tasks == 0 {
                break;
            }
            std::thread::yield_now();
        }
    }

    /// Returns a future that completes when all tasks have finished.
    ///
    /// Unlike [`drain()`](Self::drain), this method returns immediately with
    /// an [`ExecutorDrain`] future that can be awaited. This allows for
    /// non-blocking waiting in async contexts.
    ///
    /// # Examples
    ///
    /// ```
    /// # use futures::executor::block_on;
    /// use some_global_executor::Executor;
    /// use some_executor::SomeExecutor;
    /// use some_executor::task::{Task, Configuration};
    ///
    /// # block_on(async {
    /// let mut executor = Executor::new("async-example".to_string(), 4);
    ///
    /// // Spawn multiple tasks
    /// for i in 0..5 {
    ///     let task = Task::without_notifications(
    ///         format!("task-{}", i),
    ///         Configuration::default(),
    ///         async move { i * 2 }
    ///     );
    ///     executor.spawn(task);
    /// }
    ///
    /// // Asynchronously wait for completion
    /// executor.drain_async().await;
    /// # });
    /// ```
    pub fn drain_async(self) -> ExecutorDrain {
        ExecutorDrain {
            executor: self
        }
    }


    fn spawn_internal(&mut self, task: Box<dyn DynSpawnedTask<Infallible>>) {
        self.imp.spawn_internal(SpawnedTask::new(task));
    }

    /// Sets this executor as the global executor.
    ///
    /// After calling this method, this executor will be used as the default
    /// for global task spawning operations throughout the application.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// let executor = Executor::new("global".to_string(), 4);
    /// executor.set_as_global_executor();
    /// // Now this executor handles global task spawning
    /// # executor.drain();
    /// ```
    pub fn set_as_global_executor(&self) {
        some_executor::global_executor::set_global_executor(self.clone_box());
    }

    /// Sets this executor as the thread-local executor.
    ///
    /// After calling this method, this executor will be used as the default
    /// for thread-local task spawning operations on the current thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use some_global_executor::Executor;
    ///
    /// let executor = Executor::new("thread-local".to_string(), 2);
    /// executor.set_as_thread_executor();
    /// // Now this executor handles thread-local task spawning
    /// # executor.drain();
    /// ```
    pub fn set_as_thread_executor(&self) {
        some_executor::thread_executor::set_thread_executor(self.clone_box());
    }
}

impl some_executor::SomeExecutor for Executor {
    type ExecutorNotifier = Infallible;

    fn spawn<F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&mut self, task: Task<F, Notifier>) -> impl Observer<Value=F::Output>
    where
        Self: Sized,
        F::Output: Send
    {
        let (spawned,observer) = task.spawn(self);
        self.spawn_internal(Box::new(spawned));
        observer
    }

    fn spawn_async<'s, F: Future + Send + 'static, Notifier: ObserverNotified<F::Output> + Send>(&'s mut self, task: Task<F, Notifier>) -> impl Future<Output=impl Observer<Value=F::Output>> + Send + 's
    where
        Self: Sized,
        F::Output: Send + Unpin,
    {
        async {
            let (spawned,observer) = task.spawn(self);
            self.spawn_internal(Box::new(spawned));
            observer
        }
    }

    fn spawn_objsafe(&mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Box<dyn Observer<Value=Box<dyn Any + Send>, Output=some_executor::observer::FinishedObservation<Box<dyn Any + Send>>> + Send>
    {
        let (spawned,observer) = task.spawn_objsafe(self);
        self.spawn_internal(Box::new(spawned));
        Box::new(observer)
    }

    fn spawn_objsafe_async<'s>(&'s mut self, task: Task<Pin<Box<dyn Future<Output=Box<dyn Any + 'static + Send>> + 'static + Send>>, Box<dyn ObserverNotified<dyn Any + Send> + Send>>) -> Box<dyn Future<Output=Box<dyn Observer<Value=Box<dyn Any + Send>, Output=some_executor::observer::FinishedObservation<Box<dyn Any + Send>>> + Send>> + 's> {
        Box::new(async {
            self.spawn_objsafe(task)
        })
    }

    fn clone_box(&self) -> Box<DynExecutor> {
        Box::new(self.clone())
    }

    fn executor_notifier(&mut self) -> Option<Self::ExecutorNotifier> {
        None
    }
}

//boilerplates









#[cfg(test)] mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use logwise::debuginternal_sync;
    use some_executor::observer::Observation;
    use some_executor::SomeExecutor;
    use some_executor::task::Configuration;
    use test_executors::async_test;
    use some_executor::observer::Observer;

    #[cfg(target_arch = "wasm32")]
    use wasm_thread as thread;
    #[cfg(not(target_arch = "wasm32"))]
    use std::thread;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);


    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn new() {
        logwise::context::Context::reset("new".to_string());
        let e = super::Executor::new("test".to_string(), 4);
        e.drain();
    }

    #[test_executors::async_test]
    async fn spawn() {
        logwise::context::Context::reset("spawn".to_string());
        let mut e = super::Executor::new("test".to_string(), 1);
        let (sender,fut) = r#continue::continuation();
        let t = some_executor::task::Task::without_notifications("test spawn".to_string(), Configuration::default(), async move {
            sender.send(1);
        });
        let _observer = e.spawn(t);
        let r = fut.await;
        assert_eq!(r,1);
    }

    #[test_executors::async_test]
    async fn poll_count() {
        logwise::context::Context::reset("poll_count".to_string());

        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                logwise::debuginternal_sync!("poll_count is polling against {count}",count=self.0);
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    self.get_mut().0 -= 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        let f = F(3);
        let mut e = super::Executor::new("poll_count".to_string(), 4);

        let task = some_executor::task::Task::without_notifications("poll_count".to_string(), Configuration::default(), f);

        let observer = e.spawn(task);
        assert_eq!(observer.observe(), Observation::Ready(()));
    }

    #[async_test]
    async fn poll_outline() {
        logwise::context::Context::reset("poll_outline".to_string());
        struct F(u32);
        impl Future for F {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.0 == 0 {
                    Poll::Ready(())
                } else {
                    let waker = cx.waker().clone();
                    self.0 -= 1;
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_millis(10));
                        waker.wake();
                    });
                    Poll::Pending
                }
            }

        }
        let f = F(10);
        let mut e = super::Executor::new("poll_count".to_string(), 4);
        let task = some_executor::task::Task::without_notifications("poll_count".to_string(), Configuration::default(), f);
        let observer = e.spawn(task);
        e.drain_async().await;
        assert_eq!(observer.observe(), Observation::Ready(()));
    }
}