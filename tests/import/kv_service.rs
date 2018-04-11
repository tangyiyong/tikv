// Copyright 2018 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use futures::{stream, Future, Stream};
use tempdir::TempDir;
use uuid::Uuid;

use grpc::{ChannelBuilder, Environment, Result, WriteFlags};
use kvproto::importpb::*;
use kvproto::importpb_grpc::*;

use tikv::config::TiKvConfig;
use tikv::import::ImportKVServer;

fn new_kv_server() -> (ImportKVServer, ImportKvClient) {
    let temp_dir = TempDir::new("test_import_kv_server").unwrap();

    let mut cfg = TiKvConfig::default();
    cfg.server.addr = "127.0.0.1:0".to_owned();
    cfg.import.import_dir = temp_dir.path().to_str().unwrap().to_owned();
    let server = ImportKVServer::new(&cfg);

    let ch = {
        let env = Arc::new(Environment::new(1));
        let addr = server.bind_addrs().first().unwrap();
        ChannelBuilder::new(env).connect(&format!("{}:{}", addr.0, addr.1))
    };
    let client = ImportKvClient::new(ch);

    (server, client)
}

#[test]
fn test_kv_service() {
    let (mut server, client) = new_kv_server();
    server.start();

    let uuid = Uuid::new_v4().as_bytes().to_vec();
    let mut head = WriteHead::new();
    head.set_uuid(uuid.clone());

    let mut m = Mutation::new();
    m.op = Mutation_OP::Put;
    m.set_key(vec![1]);
    m.set_value(vec![1]);
    let mut batch = WriteBatch::new();
    batch.set_commit_ts(123);
    batch.mut_mutations().push(m);

    let mut open = OpenRequest::new();
    open.set_uuid(uuid.clone());

    let mut close = CloseRequest::new();
    close.set_uuid(uuid.clone());

    // Write an engine before it is opened.
    let resp = send_write(&client, &head, &batch).unwrap();
    assert!(resp.get_error().has_engine_not_found());

    // Close an engine before it it opened.
    let resp = client.close(&close).unwrap();
    assert!(resp.get_error().has_engine_not_found());

    client.open(&open).unwrap();
    let resp = send_write(&client, &head, &batch).unwrap();
    assert!(!resp.has_error());
    let resp = send_write(&client, &head, &batch).unwrap();
    assert!(!resp.has_error());
    let resp = client.close(&close).unwrap();
    assert!(!resp.has_error());

    server.shutdown();
}

fn send_write(
    client: &ImportKvClient,
    head: &WriteHead,
    batch: &WriteBatch,
) -> Result<WriteResponse> {
    let mut r1 = WriteRequest::new();
    r1.set_head(head.clone());
    let mut r2 = WriteRequest::new();
    r2.set_batch(batch.clone());
    let mut r3 = WriteRequest::new();
    r3.set_batch(batch.clone());
    let reqs: Vec<_> = vec![r1, r2, r3]
        .into_iter()
        .map(|r| (r, WriteFlags::default()))
        .collect();
    let (tx, rx) = client.write().unwrap();
    let stream = stream::iter_ok(reqs);
    stream.forward(tx).and_then(|_| rx).wait()
}
