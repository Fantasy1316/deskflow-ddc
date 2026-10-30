use super::*;
#[tokio::test]
async fn suspend_releases_listener_and_stale_endpoints_without_destroying_resumable_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let receive_dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("file.zip");
    std::fs::write(&file, b"test content").unwrap();
    let sender = Manager::new(directory.path().into()).unwrap();
    let receiver = Manager::new(receive_dir.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let address = "127.0.0.1".parse().unwrap();
    let id = sender
        .prepare(vec![file], peer, "peer".into())
        .await
        .unwrap();
    let invitation = sender.offer(id, peer, address).await.unwrap();
    let port = invitation.port;
    receiver
        .incoming(invitation, address, peer, "peer".into())
        .unwrap();
    sender.suspend().await;
    receiver.suspend().await;
    assert!(sender.tasks.lock().unwrap().is_empty());
    assert!(!sender.job(id).unwrap().running.load(Ordering::Acquire));
    let released = TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    assert_eq!(sender.snapshot()[0].status, "paused");
    assert!(sender.snapshot()[0]
        .error
        .as_deref()
        .unwrap()
        .contains("仅切换显示器"));
    assert!(receiver.job(id).unwrap().endpoint.lock().unwrap().is_none());
    assert!(receiver.accept(id, None).await.is_err());
    drop(released);
    assert!(sender.offer(id, peer, address).await.is_err());
    assert!(sender
        .prepare(vec![source.path().join("file.zip")], peer, "peer".into())
        .await
        .is_err());
    // Returning to full sharing can offer the saved job again.
    sender.enable();
    sender.offer(id, peer, address).await.unwrap();
    assert_eq!(sender.snapshot()[0].status, "waiting");
    sender.shutdown().await;
    receiver.shutdown().await;
}
#[test]
fn rejects_unsafe_names_and_manifest_conflicts() {
    use manifest::*;
    for name in ["../x", "a\\b", "CON.txt", "x:", "Lpt1", "file.", " file "] {
        assert!(safe_name(name).is_err(), "{name}");
    }
    let entry = |name: &str| Entry {
        path: name.into(),
        directory: false,
        size: 1,
        modified: "1".into(),
    };
    assert!(validate(&[entry("a"), entry("A")]).is_err());
    assert!(validate(&[entry("dir/a")]).is_err());
    assert!(validate(&[entry("../a")]).is_err());
}

pub(super) fn receiver_record(m: &Manifest, id: Uuid, target: PathBuf) -> Record {
    let offer = Offer {
        id,
        label: "files".into(),
        entries: m.sources.len(),
        bytes: m.bytes,
        hash: m.hash.clone(),
    };
    Record {
        view: View {
            id,
            label: "files".into(),
            direction: "receive".into(),
            status: "pending".into(),
            peer_name: "peer".into(),
            entries: offer.entries,
            total_bytes: offer.bytes,
            completed_bytes: 0,
            current_file: String::new(),
            destination: None,
            error: None,
            performance: metrics::View::default(),
            created_at: chrono::Utc::now().to_rfc3339(),
            conflict: None,
        },
        peer: Uuid::new_v4(),
        offer,
        manifest: None,
        target: Some(target),
    }
}
async fn exchange(
    m: Manifest,
    r: Record,
    cancel: CancellationToken,
    progress: stream::Progress,
) -> (Result<()>, Result<PathBuf>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let o = r.offer.clone();
    let sender = tokio::spawn(async move {
        let (s, _) = listener.accept().await?;
        let mut w = wire::Wire::handshake(s, &[7; 32], true).await?;
        stream::send(
            &mut w,
            &o,
            &m,
            &CancellationToken::new(),
            Arc::new(|_, _| {}),
        )
        .await
    });
    let mut w = wire::Wire::handshake(TcpStream::connect(addr).await.unwrap(), &[7; 32], false)
        .await
        .unwrap();
    let journal_dir = r.target.as_ref().unwrap().join(".test-receipts");
    std::fs::create_dir_all(&journal_dir).unwrap();
    let options = delivery::Options {
        journal: journal_dir.join(format!("{}.delivery", r.offer.id)),
        resolve: Arc::new(|_| Box::pin(async { Ok(Decision::Replace) })),
    };
    let result = tokio::select! {biased;_=cancel.cancelled()=>Err(anyhow::anyhow!("test interruption")),r=stream::receive(&mut w,&r,&cancel,progress,Arc::new(|_| {}),&options)=>r};
    drop(w);
    (sender.await.unwrap(), result)
}
#[tokio::test]
async fn streams_directories_empty_files_and_archive_without_overwriting() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let root = source.path().join("资料");
    std::fs::create_dir_all(root.join("空目录")).unwrap();
    let data = vec![0xA5; wire::CHUNK * 7 + 123];
    std::fs::write(root.join("data.zip"), &data).unwrap();
    std::fs::write(root.join("empty"), []).unwrap();
    std::fs::write(target.path().join("data.zip"), b"original").unwrap();
    let m = manifest::scan(vec![root], &CancellationToken::new()).unwrap();
    let r = receiver_record(&m, Uuid::new_v4(), target.path().into());
    let (sent, got) = exchange(
        m.clone(),
        r.clone(),
        CancellationToken::new(),
        Arc::new(|_, _| {}),
    )
    .await;
    sent.unwrap();
    let folder = got.unwrap();
    assert_eq!(folder, target.path());
    assert!(!folder.join(format!("Deskflow-{}", r.offer.id)).exists());
    assert_eq!(std::fs::read(folder.join("资料/data.zip")).unwrap(), data);
    assert!(folder.join("资料/空目录").is_dir());
    assert_eq!(
        std::fs::metadata(folder.join("资料/empty")).unwrap().len(),
        0
    );
    assert_eq!(
        std::fs::read(target.path().join("data.zip")).unwrap(),
        b"original"
    );
    assert!(!stream::stage_path(&r).unwrap().exists());
    // Lost completion ACK/restart: durable receipt must make the same operation idempotent.
    let (sent, got) = exchange(m, r, CancellationToken::new(), Arc::new(|_, _| {})).await;
    sent.unwrap();
    assert_eq!(got.unwrap(), folder);
}
#[tokio::test]
async fn resumes_after_interruption_and_repairs_a_corrupted_prefix() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let file = source.path().join("large.bin");
    let data: Vec<u8> = (0..wire::CHUNK * 12 + 31)
        .map(|n| (n % 251) as u8)
        .collect();
    std::fs::write(&file, &data).unwrap();
    let m = manifest::scan(vec![file], &CancellationToken::new()).unwrap();
    let r = receiver_record(&m, Uuid::new_v4(), target.path().into());
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let (_, got) = exchange(
        m.clone(),
        r.clone(),
        cancel,
        Arc::new(move |done, _| {
            if done >= 2 * wire::CHUNK as u64 {
                trigger.cancel();
            }
        }),
    )
    .await;
    assert!(got.is_err());
    assert!(!target.path().join("large.bin").exists());
    let partial = stream::stage_path(&r).unwrap().join("files/large.bin");
    assert!(partial.metadata().unwrap().len() >= 2 * wire::CHUNK as u64);
    // Successful resume starts from the existing, hash-verified prefix.
    let first = Arc::new(Mutex::new(None));
    let seen = first.clone();
    let (sent, got) = exchange(
        m.clone(),
        r.clone(),
        CancellationToken::new(),
        Arc::new(move |done, _| {
            let mut first = seen.lock().unwrap();
            if first.is_none() {
                *first = Some(done);
            }
        }),
    )
    .await;
    sent.unwrap();
    assert_eq!(std::fs::read(got.unwrap().join("large.bin")).unwrap(), data);
    assert!(first.lock().unwrap().unwrap() > 0);
    // Separate batch with a damaged prefix: restart this file instead of appending corrupt data.
    let r = receiver_record(&m, Uuid::new_v4(), target.path().into());
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let _ = exchange(
        m.clone(),
        r.clone(),
        cancel,
        Arc::new(move |done, _| {
            if done >= wire::CHUNK as u64 {
                trigger.cancel();
            }
        }),
    )
    .await;
    let path = stream::stage_path(&r).unwrap().join("files/large.bin");
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .write_all(b"damage")
        .unwrap();
    let (sent, got) = exchange(m, r, CancellationToken::new(), Arc::new(|_, _| {})).await;
    sent.unwrap();
    assert_eq!(std::fs::read(got.unwrap().join("large.bin")).unwrap(), data);
}
#[tokio::test]
async fn refuses_changed_source_and_keeps_incomplete_files_private() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let path = source.path().join("changing.bin");
    std::fs::write(&path, b"before").unwrap();
    let m = manifest::scan(vec![path.clone()], &CancellationToken::new()).unwrap();
    std::fs::write(path, b"after and larger").unwrap();
    let r = receiver_record(&m, Uuid::new_v4(), target.path().into());
    let final_path = target.path().join("changing.bin");
    let (sent, got) = exchange(m, r, CancellationToken::new(), Arc::new(|_, _| {})).await;
    assert!(sent.is_err() && got.is_err());
    assert!(!final_path.exists());
}
#[tokio::test]
async fn independent_wire_authenticates_and_rejects_oversized_frames() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    let s = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        wire::Wire::handshake(s, &[1; 32], true).await.is_err()
    });
    assert!(
        wire::Wire::handshake(TcpStream::connect(a).await.unwrap(), &[2; 32], false)
            .await
            .is_err()
    );
    assert!(s.await.unwrap());
    // The frame length is rejected before allocating any requested payload.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    let s = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut nonce = [0; 32];
        s.read_exact(&mut nonce).await.unwrap();
        s.write_all(&[3; 32]).await.unwrap();
        s.write_u32(u32::MAX).await.unwrap();
    });
    assert!(
        wire::Wire::handshake(TcpStream::connect(a).await.unwrap(), &[3; 32], false)
            .await
            .is_err()
    );
    s.await.unwrap();
}
async fn wait_status(m: &Manager, id: Uuid, status: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if m.snapshot()
                .iter()
                .any(|j| j.id == id && j.status == status)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn peer_decline_releases_listener_and_concurrency_slot_without_accepting_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("file.zip");
    std::fs::write(&file, [1, 2, 3]).unwrap();
    let sender = Manager::new(directory.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let mut ids = Vec::new();
    let mut port = 0;
    for index in 0..5 {
        let id = sender
            .prepare(vec![file.clone()], peer, "receiver".into())
            .await
            .unwrap();
        ids.push(id);
        let offer = sender.offer(id, peer, "127.0.0.1".parse().unwrap()).await;
        if index < 4 {
            port = offer.unwrap().port;
        } else {
            assert!(offer.err().unwrap().to_string().contains("4 个"));
        }
    }
    let declined = ids[3];
    assert!(sender
        .cancel_from_peer(declined, Uuid::new_v4())
        .await
        .is_err());
    assert!(sender
        .job(declined)
        .unwrap()
        .running
        .load(Ordering::Acquire));
    sender.cancel_from_peer(declined, peer).await.unwrap();
    assert!(!sender
        .job(declined)
        .unwrap()
        .running
        .load(Ordering::Acquire));
    assert_eq!(
        sender
            .job(declined)
            .unwrap()
            .record
            .lock()
            .unwrap()
            .view
            .status,
        "cancelled"
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    sender
        .offer(ids[4], peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    sender.remove(declined).unwrap();
    sender.cancel_from_peer(declined, peer).await.unwrap();
    sender.shutdown().await;
}

#[tokio::test]
async fn manager_requires_consent_persists_tasks_and_authenticates_original_peer() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let f = source.path().join("test.zip");
    std::fs::write(&f, vec![9; wire::CHUNK * 3]).unwrap();
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let other = Uuid::new_v4();
    let id = sender
        .prepare(vec![f], peer, "receiver".into())
        .await
        .unwrap();
    assert!(sender
        .offer(id, other, "127.0.0.1".parse().unwrap())
        .await
        .is_err());
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    let secret = invite.secret.clone();
    receiver
        .incoming(
            invite.clone(),
            "127.0.0.1".parse().unwrap(),
            other,
            "sender".into(),
        )
        .unwrap();
    assert!(receiver
        .incoming(
            invite,
            "127.0.0.1".parse().unwrap(),
            peer,
            "intruder".into()
        )
        .is_err());
    assert_eq!(receiver.snapshot()[0].status, "pending");
    assert!(std::fs::read_dir(dest.path()).unwrap().next().is_none());
    assert!(
        !std::fs::read_to_string(b.path().join(format!("{id}.json")))
            .unwrap()
            .contains(&secret)
    );
    receiver.accept(id, Some(dest.path().into())).await.unwrap();
    wait_status(&receiver, id, "complete").await;
    wait_status(&sender, id, "complete").await;
    sender.shutdown().await;
    receiver.shutdown().await;
    let reloaded = Manager::new(b.path().into()).unwrap();
    assert_eq!(reloaded.snapshot()[0].status, "complete");
    assert!(reloaded.destination(id).unwrap().join("test.zip").is_file());
    reloaded.remove(id).unwrap();
    assert!(reloaded.snapshot().is_empty());
}
#[test]
fn supports_u64_file_sizes_without_reading_entire_file_and_rejects_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("sparse.bin");
    std::fs::File::create(&p)
        .unwrap()
        .set_len(5u64 * 1024 * 1024 * 1024 + 1)
        .unwrap();
    let m = manifest::scan(vec![p], &CancellationToken::new()).unwrap();
    assert_eq!(m.bytes, 5u64 * 1024 * 1024 * 1024 + 1);
    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(manifest::scan(vec![link], &CancellationToken::new()).is_err());
    }
}

#[tokio::test]
async fn restart_resumes_same_task_and_cancel_cleans_only_its_cache() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("large.bin");
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&path).unwrap();
        for _ in 0..64 {
            file.write_all(&vec![42; wire::CHUNK]).unwrap();
        }
    }
    let peer = Uuid::new_v4();
    let other = Uuid::new_v4();
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    let id = sender
        .prepare(vec![path.clone()], peer, "receiver".into())
        .await
        .unwrap();
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), other, "sender".into())
        .unwrap();
    receiver.accept(id, Some(dest.path().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while receiver.snapshot()[0].completed_bytes < wire::CHUNK as u64 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    receiver.pause(id).unwrap();
    wait_status(&receiver, id, "paused").await;
    sender.shutdown().await;
    receiver.shutdown().await;
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    assert!(receiver.accept(id, None).await.is_err()); // Keys/endpoints are deliberately not journaled.
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), other, "sender".into())
        .unwrap();
    receiver.accept(id, None).await.unwrap();
    wait_status(&receiver, id, "complete").await;
    wait_status(&sender, id, "complete").await;
    let final_dir = receiver.destination(id).unwrap();
    assert_eq!(
        std::fs::read(final_dir.join("large.bin")).unwrap(),
        std::fs::read(&path).unwrap()
    );
    // Cancel a different partially received task; never remove previous results.
    let second = sender
        .prepare(vec![path], peer, "receiver".into())
        .await
        .unwrap();
    let invite = sender
        .offer(second, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), other, "sender".into())
        .unwrap();
    let job = receiver.job(second).unwrap();
    job.record.lock().unwrap().target = Some(dest.path().into());
    let stage = stream::stage_path(&job.record.lock().unwrap()).unwrap();
    std::fs::create_dir(&stage).unwrap();
    std::fs::write(stage.join("unfinished"), b"partial").unwrap();
    receiver.cancel(second).await.unwrap();
    assert!(!stage.exists());
    assert!(final_dir.join("large.bin").is_file());
    receiver.pause(id).unwrap();
    receiver.cancel(id).await.unwrap();
    assert_eq!(
        receiver.job(id).unwrap().record.lock().unwrap().view.status,
        "complete"
    );
    sender.shutdown().await;
    receiver.shutdown().await;
}

#[tokio::test]
async fn changed_source_pauses_sender_instead_of_waiting_forever() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("changed");
    std::fs::write(&file, b"before").unwrap();
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let id = sender
        .prepare(vec![file.clone()], peer, "peer".into())
        .await
        .unwrap();
    std::fs::write(&file, b"after and larger").unwrap();
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), peer, "peer".into())
        .unwrap();
    receiver.accept(id, Some(dest.path().into())).await.unwrap();
    wait_status(&sender, id, "paused").await;
    assert!(sender.snapshot()[0]
        .error
        .as_ref()
        .unwrap()
        .contains("源文件"));
    assert!(!dest.path().join("changed").exists());
    sender.shutdown().await;
    receiver.shutdown().await;
}

#[test]
fn corrupt_journal_does_not_prevent_application_startup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.json", Uuid::new_v4()));
    std::fs::write(&path, b"{partial").unwrap();
    let manager = Manager::new(dir.path().into()).unwrap();
    assert!(manager.snapshot().is_empty());
    assert_eq!(manager.warnings().len(), 1);
    assert_eq!(std::fs::read(path).unwrap(), b"{partial");
}

#[tokio::test]
async fn insufficient_space_fails_before_receiving_file_contents() {
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let file = source.path().join("too-large.bin");
    let available = disk::available(dest.path()).unwrap();
    std::fs::File::create(&file)
        .unwrap()
        .set_len(available + 2 * 1024 * 1024)
        .unwrap();
    let manifest = manifest::scan(vec![file], &CancellationToken::new()).unwrap();
    let record = receiver_record(&manifest, Uuid::new_v4(), dest.path().into());
    let (_, received) = exchange(
        manifest,
        record.clone(),
        CancellationToken::new(),
        Arc::new(|_, _| panic!("No file contents should be received")),
    )
    .await;
    assert!(received.unwrap_err().to_string().contains("空间不足"));
    assert_eq!(
        std::fs::metadata(
            stream::stage_path(&record)
                .unwrap()
                .join("files/too-large.bin")
        )
        .unwrap()
        .len(),
        0
    );
    assert!(!dest.path().join("too-large.bin").exists());
}

#[test]
fn publication_never_replaces_existing_directory_even_if_it_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&target).unwrap();
    std::fs::write(source.join("file"), b"verified").unwrap();
    assert!(commit::publish(&source, &target).is_err());
    assert!(std::fs::read_dir(&target).unwrap().next().is_none());
    assert!(source.join("file").is_file());
    std::fs::remove_dir(&target).unwrap();
    commit::publish(&source, &target).unwrap();
    assert_eq!(std::fs::read(target.join("file")).unwrap(), b"verified");
}

#[tokio::test]
async fn authenticated_keepalives_are_ignored_before_the_next_data_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut wire = wire::Wire::handshake(socket, &[11; 32], true)
            .await
            .unwrap();
        wire.send(&[]).await.unwrap();
        wire.send(b"next data").await.unwrap();
        wire.metrics.snapshot()
    });
    let mut wire =
        wire::Wire::handshake(TcpStream::connect(address).await.unwrap(), &[11; 32], false)
            .await
            .unwrap();
    assert_eq!(wire.receive().await.unwrap(), b"next data");
    assert!(server.await.unwrap().sent_bytes >= 9);
    assert!(wire.metrics.snapshot().received_bytes >= 9);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "manual release-mode throughput diagnostic; writes 128 MiB"]
async fn full_pipeline_throughput_diagnostic() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let file = source.path().join("throughput.bin");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&file).unwrap();
        let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
        for _ in 0..128 {
            f.write_all(&bytes).unwrap();
        }
    }
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let id = sender
        .prepare(vec![file], peer, "peer".into())
        .await
        .unwrap();
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), peer, "peer".into())
        .unwrap();
    let start = std::time::Instant::now();
    receiver
        .accept(id, Some(destination.path().into()))
        .await
        .unwrap();
    wait_status(&receiver, id, "complete").await;
    wait_status(&sender, id, "complete").await;
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "128 MiB full encrypted loopback + SHA256 + sync: {seconds:.3}s, {:.1} MiB/s",
        128.0 / seconds
    );
    let sent = sender.snapshot();
    let received = receiver.snapshot();
    println!(
        "sender: {}",
        serde_json::to_string(&sent[0].performance).unwrap()
    );
    println!(
        "receiver: {}",
        serde_json::to_string(&received[0].performance).unwrap()
    );
    assert!(sent[0].performance.sent_bytes >= 128 * 1024 * 1024);
    assert!(received[0].performance.received_bytes >= 128 * 1024 * 1024);
    sender.shutdown().await;
    receiver.shutdown().await;
}

#[tokio::test]
async fn conflict_choices_save_directly_replace_whole_directories_or_stop_without_changes() {
    for decision in [Decision::Replace, Decision::KeepBoth, Decision::Stop] {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let folder = source.path().join("资料");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("new.txt"), b"incoming").unwrap();
        let extra = source.path().join("first.txt");
        std::fs::write(&extra, b"also incoming").unwrap();
        std::fs::create_dir(target.path().join("资料")).unwrap();
        std::fs::write(target.path().join("资料/old.txt"), b"original").unwrap();
        let sender = Manager::new(a.path().into()).unwrap();
        let receiver = Manager::new(b.path().into()).unwrap();
        let peer = Uuid::new_v4();
        let id = sender
            .prepare(vec![extra, folder], peer, "peer".into())
            .await
            .unwrap();
        let invite = sender
            .offer(id, peer, "127.0.0.1".parse().unwrap())
            .await
            .unwrap();
        receiver
            .incoming(invite, "127.0.0.1".parse().unwrap(), peer, "peer".into())
            .unwrap();
        receiver
            .accept(id, Some(target.path().into()))
            .await
            .unwrap();
        wait_status(&receiver, id, "conflict").await;
        let conflict = receiver.snapshot()[0].conflict.clone().unwrap();
        assert_eq!(conflict.name, "资料");
        assert!(conflict.directory && conflict.existing_directory);
        assert!(
            !target.path().join("first.txt").exists(),
            "no files publish before all choices"
        );
        assert!(receiver
            .resolve_conflict(id, Uuid::new_v4(), Decision::Replace)
            .is_err());
        receiver
            .resolve_conflict(id, conflict.token, decision)
            .unwrap();
        assert!(receiver
            .resolve_conflict(id, conflict.token, Decision::Replace)
            .is_err());
        if matches!(decision, Decision::Stop) {
            wait_status(&receiver, id, "paused").await;
            assert_eq!(
                std::fs::read(target.path().join("资料/old.txt")).unwrap(),
                b"original"
            );
            assert!(!target.path().join("资料/new.txt").exists());
            assert!(!target.path().join("first.txt").exists());
            let record = receiver.job(id).unwrap().record.lock().unwrap().clone();
            assert!(stream::stage_path(&record).unwrap().exists());
            receiver.cancel(id).await.unwrap();
            assert!(!stream::stage_path(&record).unwrap().exists());
        } else {
            wait_status(&receiver, id, "complete").await;
            wait_status(&sender, id, "complete").await;
            assert_eq!(
                receiver.destination(id).unwrap(),
                std::fs::canonicalize(target.path()).unwrap()
            );
            assert_eq!(
                std::fs::read(target.path().join("first.txt")).unwrap(),
                b"also incoming"
            );
            if matches!(decision, Decision::Replace) {
                assert!(!target.path().join("资料/old.txt").exists());
                assert_eq!(
                    std::fs::read(target.path().join("资料/new.txt")).unwrap(),
                    b"incoming"
                );
            } else {
                assert_eq!(
                    std::fs::read(target.path().join("资料/old.txt")).unwrap(),
                    b"original"
                );
                assert_eq!(
                    std::fs::read(target.path().join("资料 (2)/new.txt")).unwrap(),
                    b"incoming"
                );
            }
            assert!(!target.path().join(format!("Deskflow-{id}")).exists());
            assert!(!target
                .path()
                .join(format!(".deskflow-ddc-{id}.partial"))
                .exists());
        }
        sender.shutdown().await;
        receiver.shutdown().await;
    }
}

#[tokio::test]
async fn pausing_a_conflict_clears_the_prompt_and_allows_a_fresh_choice() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let file = source.path().join("archive.zip");
    std::fs::write(&file, b"new").unwrap();
    std::fs::write(target.path().join("archive.zip"), b"old").unwrap();
    std::fs::write(target.path().join("archive (2).zip"), b"also old").unwrap();
    let sender = Manager::new(a.path().into()).unwrap();
    let receiver = Manager::new(b.path().into()).unwrap();
    let peer = Uuid::new_v4();
    let id = sender
        .prepare(vec![file], peer, "peer".into())
        .await
        .unwrap();
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), peer, "peer".into())
        .unwrap();
    receiver
        .accept(id, Some(target.path().into()))
        .await
        .unwrap();
    wait_status(&receiver, id, "conflict").await;
    let old_token = receiver.snapshot()[0].conflict.as_ref().unwrap().token;
    receiver.pause(id).unwrap();
    wait_status(&receiver, id, "paused").await;
    assert!(receiver.snapshot()[0].conflict.is_none());
    assert!(receiver
        .resolve_conflict(id, old_token, Decision::Replace)
        .is_err());
    wait_status(&sender, id, "waiting").await;
    assert!(sender.snapshot()[0]
        .error
        .as_deref()
        .unwrap()
        .contains("连接中断"));
    receiver.accept(id, None).await.unwrap();
    wait_status(&receiver, id, "conflict").await;
    let sending = sender.snapshot().remove(0);
    assert_eq!(sending.status, "transferring");
    assert!(
        sending.error.is_none(),
        "恢复传输后仍保留旧提示：{:?}",
        sending.error
    );
    assert!(receiver.snapshot()[0].error.is_none());
    let token = receiver.snapshot()[0].conflict.as_ref().unwrap().token;
    assert_ne!(old_token, token);
    receiver
        .resolve_conflict(id, token, Decision::KeepBoth)
        .unwrap();
    wait_status(&receiver, id, "complete").await;
    wait_status(&sender, id, "complete").await;
    assert_eq!(
        std::fs::read(target.path().join("archive (3).zip")).unwrap(),
        b"new"
    );
    assert_eq!(
        std::fs::read(target.path().join("archive.zip")).unwrap(),
        b"old"
    );
    assert_eq!(
        std::fs::read(target.path().join("archive (2).zip")).unwrap(),
        b"also old"
    );
    // Reoffer after a lost final ACK: the receipt is outside the selected folder,
    // so no second copy or second conflict prompt is created.
    sender.job(id).unwrap().record.lock().unwrap().view.status = "paused".into();
    let invite = sender
        .offer(id, peer, "127.0.0.1".parse().unwrap())
        .await
        .unwrap();
    receiver
        .incoming(invite, "127.0.0.1".parse().unwrap(), peer, "peer".into())
        .unwrap();
    receiver.accept(id, None).await.unwrap();
    wait_status(&receiver, id, "complete").await;
    wait_status(&sender, id, "complete").await;
    assert!(!target.path().join("archive (4).zip").exists());
    sender.shutdown().await;
    receiver.shutdown().await;
}
