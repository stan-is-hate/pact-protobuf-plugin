use std::panic::catch_unwind;
use std::path::Path;
use base64::Engine;
use bytes::{BufMut, BytesMut};

use expectest::prelude::*;
use pact_consumer::mock_server::StartMockServerAsync;
use pact_consumer::prelude::PactBuilderAsync;
use prost::encoding::WireType;
use prost::Message;
use prost_types::FileDescriptorSet;
use serde_json::json;
use tonic::Request;
use tower::ServiceExt;
use pact_protobuf_plugin::dynamic_message::{DynamicMessage, PactCodec};
use pact_protobuf_plugin::message_decoder::{ProtobufField, ProtobufFieldData};
use pact_protobuf_plugin::utils::{find_message_descriptor_for_type};

async fn mock_server_block() {
  let mut pact_builder = PactBuilderAsync::new_v4("null-and-void", "protobuf-plugin");
  let _mock_server = pact_builder
    .using_plugin("protobuf", None).await
    .synchronous_message_interaction("doesn't matter, won't be called", |mut i| async move {
      let proto_file = Path::new("tests/simple.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:content-type": "application/protobuf",
        "pact:proto-service": "Test/GetTest",

        "request": {
          "in": "matching(boolean, true)"
        },

        "response": {
          "out": "matching(boolean, true)"
        }
      })).await;
      i
    })
    .await
    .start_mock_server_async(Some("protobuf/transport/grpc"))
    .await;

  // Should fail as we have not made a request to the mock server when the mock server is dropped
  // at the end of this function
}

#[test_log::test]
fn mock_server_with_no_requests() {
    let result = catch_unwind(|| {
      let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("new runtime");
      runtime.block_on(mock_server_block())
    });

    let error = result.unwrap_err();
    let error_message = panic_message::panic_message(&error);
    expect!(error_message).to(be_equal_to(
      "plugin mock server failed verification:\n    1) Test/GetTest: Did not receive any requests for path 'Test/GetTest'\n"));
}

#[test_log::test(tokio::test(flavor = "multi_thread"))]
async fn each_value_matcher() {
  let mut pact_builder = PactBuilderAsync::new_v4("each-value", "protobuf-plugin");
  pact_builder
    .using_plugin("protobuf", None).await
    .synchronous_message_interaction("get a list of values", |mut i| async move {
      let proto_file = Path::new("tests/simple.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:content-type": "application/protobuf",
        "pact:proto-service": "Test/GetValues",

        "request": {
          "value": "eachValue(matching(type, '00000000000000000000000000000000'))"
        },

        "response": {
          "value": "eachValue(matching(type, '00000000000000000000000000000000'))"
        }
      })).await;
      i
    })
    .await;
  let mock_server = pact_builder
    .start_mock_server_async(Some("protobuf/transport/grpc"))
    .await;

  let url = mock_server.url();
  // unlike what's under /tests dir, this encoded descriptor does NOT have a package specified
  // somehow it's still compatible cause the test passes
  let descriptors = base64::engine::general_purpose::STANDARD.decode(
    "CogCCgxzaW1wbGUucHJvdG8iGwoJTWVzc2FnZUluEg4KAmluGAEgASgIUgJpbiIeCgpNZXNzYWdlT3V0EhAKA291\
    dBgBIAEoCFIDb3V0IicKD1ZhbHVlc01lc3NhZ2VJbhIUCgV2YWx1ZRgBIAMoCVIFdmFsdWUiKAoQVmFsdWVzTWVzc2FnZU\
    91dBIUCgV2YWx1ZRgBIAMoCVIFdmFsdWUyYAoEVGVzdBIkCgdHZXRUZXN0EgouTWVzc2FnZUluGgsuTWVzc2FnZU91dCIA\
    EjIKCUdldFZhbHVlcxIQLlZhbHVlc01lc3NhZ2VJbhoRLlZhbHVlc01lc3NhZ2VPdXQiAGIGcHJvdG8z").unwrap();
  let fds = FileDescriptorSet::decode(descriptors.as_slice()).unwrap();
  let field = ProtobufField {
    field_num: 1,
    field_name: "value".to_string(),
    wire_type: WireType::LengthDelimited,
    data: ProtobufFieldData::String("value1".to_string())
  };
  let field2 = ProtobufField {
    field_num: 1,
    field_name: "value".to_string(),
    wire_type: WireType::LengthDelimited,
    data: ProtobufFieldData::String("value2".to_string())
  };
  let fields = vec![ field, field2 ];
  let message = DynamicMessage::new(fields.as_slice(), &fds);
  let mut buffer = BytesMut::new();
  buffer.put_u8(0);
  message.write_to(&mut buffer).unwrap();

  let mut conn = tonic::transport::Endpoint::from_shared(url.to_string())
    .unwrap()
    .connect()
    .await
    .unwrap();
  conn.ready().await.unwrap();

  let (input_message, _) = find_message_descriptor_for_type(".ValuesMessageIn", &fds).unwrap();
  // searching by name without package next, to confirm we're backwards compatible 
  // (it's verified by unit tests too, but wouldn't hurt to check here as well)
  let (output_message, _) = find_message_descriptor_for_type("ValuesMessageOut", &fds).unwrap();
  let interaction = pact_builder.build()
    .interactions().first().unwrap()
    .as_v4_sync_message().unwrap();

  let codec = PactCodec::new(&fds, &input_message, &output_message, &interaction);
  let mut grpc = tonic::client::Grpc::new(conn);
  let path = http::uri::PathAndQuery::try_from("/Test/GetValues").unwrap();
  grpc.unary(Request::new(message), path, codec).await.unwrap();
}

