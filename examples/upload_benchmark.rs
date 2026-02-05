//! SFTP Upload Benchmark
//!
//! Measures upload throughput for various file sizes and chunk sizes.
//!
//! Usage:
//!   cargo run --example upload_benchmark -- --host <hostname> --user <username> --key <path>
//!
//! Examples:
//!   # Basic benchmark with default sizes
//!   cargo run --example upload_benchmark -- --host rhel10 --user mm --key ~/.ssh/id_ed25519
//!
//!   # Test specific file sizes
//!   cargo run --example upload_benchmark -- --host rhel10 --user mm --key ~/.ssh/id_ed25519 --sizes 10,50
//!
//!   # Compare different chunk sizes (in KB)
//!   cargo run --example upload_benchmark -- --host rhel10 --user mm --key ~/.ssh/id_ed25519 --sizes 50 --chunk-sizes 32,64,128,256,512
//!
//!   # Quick single test
//!   cargo run --example upload_benchmark -- --host rhel10 --user mm --key ~/.ssh/id_ed25519 --sizes 10 --iterations 1

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;

// Re-use the SSH session from the main crate
use rotomate::ssh::{AuthMethod, DEFAULT_CHUNK_SIZE, Session};

#[derive(Parser, Debug)]
#[command(name = "upload_benchmark")]
#[command(about = "Benchmark SFTP upload throughput")]
struct Args {
    /// Remote hostname
    #[arg(long)]
    host: String,

    /// SSH username
    #[arg(long, default_value = "root")]
    user: String,

    /// Path to SSH private key
    #[arg(long)]
    key: PathBuf,

    /// SSH port
    #[arg(long, default_value = "22")]
    port: u16,

    /// File sizes to test in MB (comma-separated)
    #[arg(long, default_value = "1,10,50,100")]
    sizes: String,

    /// Number of iterations per test
    #[arg(long, default_value = "3")]
    iterations: u32,

    /// Remote directory for uploads
    #[arg(long, default_value = "/tmp")]
    remote_dir: String,

    /// Test different chunk sizes in KB (comma-separated)
    /// If specified, runs tests with each chunk size for comparison
    #[arg(long)]
    chunk_sizes: Option<String>,
}

#[derive(Clone)]
struct BenchmarkResult {
    size_mb: u64,
    chunk_kb: usize,
    duration: Duration,
    throughput_mbps: f64,
}

fn format_throughput(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.2} GB/s", mbps / 1000.0)
    } else {
        format!("{:.2} MB/s", mbps)
    }
}

fn format_duration(d: Duration) -> String {
    if d.as_secs() > 0 {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{:.0}ms", d.as_millis())
    }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.2} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

async fn run_upload_test(
    host: &str,
    port: u16,
    key_path: &PathBuf,
    user: &str,
    local_file: &std::path::Path,
    remote_dir: &str,
    chunk_size: usize,
) -> Result<(u64, Duration)> {
    let mut session = Session::connect(
        AuthMethod::PublicKey(key_path.clone()),
        user,
        (host, port),
        300, // 5 minute timeout
    )
    .await?;

    let remote_path = format!("{}/benchmark_test.bin", remote_dir);

    let start = Instant::now();
    let bytes_written = session
        .copy_file_to_remote(local_file, &remote_path, chunk_size)
        .await?;
    let duration = start.elapsed();

    // Clean up remote file
    let _ = session.delete_remote_file(&remote_path).await;
    let _ = session.close().await;

    Ok((bytes_written, duration))
}

fn create_test_file(path: &std::path::Path, size: u64) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::File::create(path)?;

    // Create a pattern that's not too compressible but fast to generate
    let chunk_size = 64 * 1024; // 64KB chunks
    let mut chunk = vec![0u8; chunk_size];

    // Fill with a pattern based on position
    for (i, byte) in chunk.iter_mut().enumerate() {
        *byte = ((i * 7 + 13) % 256) as u8;
    }

    let mut written = 0u64;
    while written < size {
        let to_write = std::cmp::min(chunk_size as u64, size - written) as usize;
        file.write_all(&chunk[..to_write])?;
        written += to_write as u64;
        chunk.rotate_left(1);
    }

    file.flush()?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    // Parse file sizes
    let sizes: Vec<u64> = args
        .sizes
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if sizes.is_empty() {
        anyhow::bail!("No valid sizes specified");
    }

    // Parse chunk sizes (in KB) or use default
    let chunk_sizes: Vec<usize> = if let Some(ref chunks) = args.chunk_sizes {
        chunks
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .map(|kb| kb * 1024) // Convert KB to bytes
            .collect()
    } else {
        vec![DEFAULT_CHUNK_SIZE] // Default 256KB
    };

    println!("SFTP Upload Benchmark");
    println!("=====================");
    println!("Host: {}:{}", args.host, args.port);
    println!("User: {}", args.user);
    println!("Key:  {}", args.key.display());
    println!("File sizes: {:?} MB", sizes);
    println!(
        "Chunk sizes: {:?} KB",
        chunk_sizes.iter().map(|c| c / 1024).collect::<Vec<_>>()
    );
    println!("Iterations: {}", args.iterations);
    println!();

    // Expand tilde in key path
    let key_path: PathBuf = shellexpand::tilde(&args.key.to_string_lossy())
        .to_string()
        .into();

    // Create temp directory for test files
    let temp_dir = std::env::temp_dir().join("upload_benchmark");
    std::fs::create_dir_all(&temp_dir)?;

    let mut all_results: Vec<BenchmarkResult> = Vec::new();

    for size_mb in &sizes {
        let test_file = temp_dir.join(format!("test_{}mb.bin", size_mb));
        let file_size = *size_mb * 1024 * 1024;

        print!("Creating {} MB test file... ", size_mb);
        std::io::Write::flush(&mut std::io::stdout())?;
        let start = Instant::now();
        create_test_file(&test_file, file_size)?;
        println!("done ({:.2}s)", start.elapsed().as_secs_f64());

        for chunk_size in &chunk_sizes {
            let chunk_kb = chunk_size / 1024;
            println!(
                "\nTesting {} MB file with {} KB chunks:",
                size_mb, chunk_kb
            );

            let mut results = Vec::new();

            for i in 1..=args.iterations {
                print!("  Iteration {}/{}: ", i, args.iterations);
                std::io::Write::flush(&mut std::io::stdout())?;

                match run_upload_test(
                    &args.host,
                    args.port,
                    &key_path,
                    &args.user,
                    &test_file,
                    &args.remote_dir,
                    *chunk_size,
                )
                .await
                {
                    Ok((bytes_written, duration)) => {
                        let throughput_mbps =
                            (bytes_written as f64 / 1_000_000.0) / duration.as_secs_f64();

                        println!(
                            "{} in {} ({}/s)",
                            format_size(bytes_written),
                            format_duration(duration),
                            format_throughput(throughput_mbps)
                        );

                        let result = BenchmarkResult {
                            size_mb: *size_mb,
                            chunk_kb,
                            duration,
                            throughput_mbps,
                        };
                        results.push(result.clone());
                        all_results.push(result);
                    }
                    Err(e) => {
                        println!("FAILED: {}", e);
                    }
                }
            }

            // Print per-configuration summary
            if !results.is_empty() {
                let avg_throughput: f64 =
                    results.iter().map(|r| r.throughput_mbps).sum::<f64>() / results.len() as f64;
                println!("  Average: {}/s", format_throughput(avg_throughput));
            }
        }

        // Clean up local test file
        let _ = std::fs::remove_file(&test_file);
    }

    // Print final summary
    println!("\n");
    println!("Summary");
    println!("=======");

    if chunk_sizes.len() > 1 {
        // Group by file size, show chunk size comparison
        println!(
            "{:>10} {:>12} {:>12} {:>12}",
            "Size", "Chunk", "Avg Time", "Avg Speed"
        );
        println!("{}", "-".repeat(50));

        for size_mb in &sizes {
            for chunk_kb in chunk_sizes.iter().map(|c| c / 1024) {
                let results: Vec<_> = all_results
                    .iter()
                    .filter(|r| r.size_mb == *size_mb && r.chunk_kb == chunk_kb)
                    .collect();

                if !results.is_empty() {
                    let avg_duration =
                        results.iter().map(|r| r.duration).sum::<Duration>() / results.len() as u32;
                    let avg_throughput: f64 =
                        results.iter().map(|r| r.throughput_mbps).sum::<f64>()
                            / results.len() as f64;

                    println!(
                        "{:>7} MB {:>9} KB {:>12} {:>12}",
                        size_mb,
                        chunk_kb,
                        format_duration(avg_duration),
                        format_throughput(avg_throughput)
                    );
                }
            }
        }
    } else {
        // Single chunk size - simpler summary
        println!(
            "{:>10} {:>12} {:>12} {:>12} {:>12}",
            "Size", "Min", "Max", "Avg", "Avg Speed"
        );
        println!("{}", "-".repeat(60));

        for size_mb in &sizes {
            let results: Vec<_> = all_results
                .iter()
                .filter(|r| r.size_mb == *size_mb)
                .collect();

            if !results.is_empty() {
                let min_duration = results.iter().map(|r| r.duration).min().unwrap();
                let max_duration = results.iter().map(|r| r.duration).max().unwrap();
                let avg_duration =
                    results.iter().map(|r| r.duration).sum::<Duration>() / results.len() as u32;
                let avg_throughput: f64 =
                    results.iter().map(|r| r.throughput_mbps).sum::<f64>() / results.len() as f64;

                println!(
                    "{:>7} MB {:>12} {:>12} {:>12} {:>12}",
                    size_mb,
                    format_duration(min_duration),
                    format_duration(max_duration),
                    format_duration(avg_duration),
                    format_throughput(avg_throughput)
                );
            }
        }
    }

    // Clean up temp directory
    let _ = std::fs::remove_dir_all(&temp_dir);

    Ok(())
}
