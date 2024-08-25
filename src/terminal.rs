use std::{
    future::Future,
    io::{stdout, Write},
    marker::PhantomData,
    time::Duration,
};

use crossterm::{terminal::Clear, ExecutableCommand};

pub struct Loading<F, T>(PhantomData<F>, PhantomData<T>)
where
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
    T: Send + 'static;

impl<F, T> Loading<F, T>
where
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    const LOADING_CHARS: [char; 8] = ['⣾', '⣷', '⣯', '⣟', '⡿', '⢿', '⣻', '⣽'];

    /// run a task for the given future, print to the screen loading indicator
    /// while the task is running, at the end returns the task result
    pub async fn start(
        future: F,
        loading_message: &str,
        complete_message: &str,
        error_message: &str,
    ) -> T {
        let mut stdout = stdout();
        let mut char_cycle = Self::LOADING_CHARS.iter().cycle();
        let task = tokio::task::spawn(future);

        while !task.is_finished() {
            let _ = write!(
                stdout,
                "{} {}\r",
                char_cycle.next().unwrap(),
                loading_message
            );
            let _ = stdout.flush();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = stdout.execute(Clear(crossterm::terminal::ClearType::CurrentLine));

        task.await
            .map(|result| {
                result
                    .and_then(|result| {
                        let _ = write!(stdout, "{}\r\n", complete_message);
                        Ok(result)
                    })
                    .expect(error_message)
            })
            .expect("corotine issue fetching terminal output")
    }
}
