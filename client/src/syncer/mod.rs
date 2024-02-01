pub mod remote;

use futures::{
    channel::mpsc::{Receiver},
    join, StreamExt,
};
use std::path::PathBuf;

use time::OffsetDateTime;
use tokio::time::Duration;
use std::sync::Arc;
use tokio::sync::Mutex;

use log::{debug, trace};

use crate::local_db::*;
use crate::models::*;
use crate::chunker::*;

const INTERVAL_CHECK_DOWNLOAD_SEC: Duration = Duration::from_secs(23);
const INTERVAL_CHECK_UPLOAD_SEC: Duration = Duration::from_secs(47);

pub async fn run(
    pool: &ConnectionPool,
    chunker: &mut Chunker<InMemoryCache>,
    remote: &remote::Remote,
    mut local_registry_updated_rx: Receiver<IndexerUpdateEvent>,
) {
    let chunker = Arc::new(Mutex::new(chunker));

    let interval_check_download = async {
        let chunker = Arc::clone(&chunker);

        loop {
            debug!("[syncer] interval scan");

            let conn = &mut pool.get().unwrap();

            let latest_local = latest_jid(conn);
            let to_download = remote.list(latest_local.unwrap_or(0)).await.unwrap();

            // TODO dowload chunks and create records in registry
            for d in &to_download {
                let chunks: Vec<&str> = d.chunk_ids.split(',').collect();
                let mut chunker = chunker.lock().await;

                for c in &chunks {
                    if !chunker.check_chunk(c).unwrap() {
                        chunker.save_chunk(c, remote.download(c).await.unwrap());
                    }
                }
                chunker.save(&d.path, chunks);

                let form = build_file_record(&d.path, d.id);
                create_file_records(conn, &vec![form]);
            }

            trace!("interval_check_download {:?} {:?}", latest_local, to_download);

            tokio::time::sleep(INTERVAL_CHECK_DOWNLOAD_SEC).await;
        }
    };

    let interval_check_upload = async {
        let chunker = Arc::clone(&chunker);

        loop {
            debug!("[syncer] interval scan");

            let conn = &mut pool.get().unwrap();

            let to_upload = updated_locally_file_records(conn);

            for f in &to_upload {
                let mut chunker = chunker.lock().await;
                let chunk_ids = chunker.hashify(&f.path).unwrap();
                let r = remote.commit(&f.path, &chunk_ids.join(","), "t").await.unwrap();

                match r {
                    remote::CommitResultStatus::Success(jid) => {
                        trace!("commited {:?}", jid);
                        update_jid_on_file_record(conn, f, jid);
                    },
                    remote::CommitResultStatus::NeedChunks(chunks) => {
                        trace!("need chunks {:?}", chunks);
                        for c in chunks.split(',') {
                            // TODO bundle multiple into one request
                            remote.upload(c, chunker.read_chunk(c).unwrap()).await;
                        }
                    },
                }
            }

            // need to wait only if we didn't upload anything
            // otherwise it should re-run immideately
            if to_upload.is_empty() {
                tokio::time::sleep(INTERVAL_CHECK_UPLOAD_SEC).await;
            }
        }
    };

    let monitor_indexer_updates = async {
        while let Some(event) = local_registry_updated_rx.next().await {
            trace!("fs event triggered {:?}", event);

            // if let Err(e) = do_sync(pool).await {
            //     // Handle the error, for example, log it
            //     error!("Error in do_sync: {}", e);
            //     break; // or continue, depending on how you want to handle errors
            // }
        }
    };

    // remote_polling to change from remote to local

    join!(interval_check_download, interval_check_upload, monitor_indexer_updates);
}

fn build_file_record(path: &str, jid: i32) -> FileRecordCreateForm {
    let metadata = PathBuf::from(path).metadata().unwrap();
    let size: i64 = metadata.len().try_into().unwrap();
    let modified_at = OffsetDateTime::from(metadata.modified().unwrap());

    FileRecordCreateForm {
        jid: Some(jid),
        path: path.to_string(),
        size,
        format: "t".to_string(),
        modified_at,
    }
}
