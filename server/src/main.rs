fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(result) = gxserver::cli::run_hook_command(&args) {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    // Before the runtime starts any thread: PATH is process-wide state.
    gxserver::prepare_process_environment();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("start the gxserver runtime");
    runtime.block_on(async {
        if let Err(error) = gxserver::cli::run(args).await {
            eprintln!("{error}");
            std::process::exit(1);
        }
    });
    // Dropping the runtime waits for every blocking task without limit, and one stuck on a hung child or drive kept finished processes alive for hours.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}
