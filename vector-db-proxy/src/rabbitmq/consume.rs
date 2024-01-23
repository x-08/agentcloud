use crate::data::chunking::{Chunking, ChunkingStrategy, PdfChunker};
use crate::data::processing_incoming_messages::process_messages;
use crate::gcp::gcs::get_object_from_gcs;
use crate::hash_map_values_as_serde_values;
use crate::rabbitmq::client::{bind_queue_to_exchange, channel_rabbitmq, connect_rabbitmq};
use crate::rabbitmq::models::RabbitConnect;
use crate::qdrant::utils::Qdrant;

use amqp_serde::types::ShortStr;
use amqprs::channel::{BasicAckArguments, BasicCancelArguments, BasicConsumeArguments};
use anyhow::Result;
use qdrant_client::client::QdrantClient;
use qdrant_client::prelude::PointStruct;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub async fn subscribe_to_queue(
    app_data: Arc<RwLock<QdrantClient>>,
    connection_details: RabbitConnect,
    exchange_name: &str,
    queue_name: &str,
    routing_key: &str,
) -> Result<()> {
    // loop {
    let mut connection = connect_rabbitmq(&connection_details).await;
    let mut channel = channel_rabbitmq(&connection).await;
    bind_queue_to_exchange(
        &mut connection,
        &mut channel,
        &connection_details,
        exchange_name,
        queue_name,
        routing_key,
    )
    .await;
    let args = BasicConsumeArguments::new(queue_name, "");
    let (ctag, mut messages_rx) = channel.basic_consume_rx(args.clone()).await.unwrap();
    while let Some(message) = messages_rx.recv().await {
        let headers = message.basic_properties.unwrap().headers().unwrap().clone();
        if let Some(stream) = headers.get(&ShortStr::try_from("stream").unwrap()) {
            let stream_string: String = stream.to_string();
            let stream_split: Vec<&str> = stream_string.split("_").collect();
            let datasource_id = stream_split.to_vec()[0];
            if let Some(msg) = message.content {
                // if the header 'type' is present then assume that it is a file upload. pull from gcs
                if let Ok(message_string) = String::from_utf8(msg.clone().to_vec()) {
                    if let Some(_) = headers.get(&ShortStr::try_from("type").unwrap()) {
                        if let Ok(_json) = serde_json::from_str(message_string.as_str()) {
                            let message_data: Value = _json;
                            if let Some(bucket_name) = message_data.get("") {
                                if let Some(file_name) = message_data.get("") {
                                    if let Ok(file) = get_object_from_gcs(
                                        bucket_name.to_string(),
                                        file_name.to_string(),
                                    )
                                    .await
                                    {
                                        let pdf = PdfChunker::default();
                                        let (document_text, metadata) = pdf
                                            .extract_text_from_pdf(file)
                                            .expect("TODO: panic message");
                                        let chunks = pdf
                                            .chunk(
                                                document_text,
                                                Some(metadata),
                                                ChunkingStrategy::SEMANTIC_CHUNKING,
                                            )
                                            .await
                                            .unwrap();
                                        for element in chunks.iter() {
                                            let mut metadata = element.metadata.clone().unwrap();
                                            metadata.insert(
                                                "text".to_string(),
                                                element.page_content.to_string(),
                                            );
                                            let embedding_vector = &element.embedding_vector;

                                            let payload: HashMap<String, Value> =
                                                hash_map_values_as_serde_values!(metadata);
                                            let qdrant_point_struct = PointStruct::new(
                                                Uuid::new_v4().to_string(),
                                                embedding_vector.to_owned().unwrap().to_owned(),
                                                json!(payload).try_into().unwrap(),
                                            );
                                            let app_data_clone = Arc::clone(&app_data);
                                            let qdrant = Qdrant::new(app_data_clone, datasource_id.to_string());
                                            qdrant.upsert_data_point(qdrant_point_struct).await?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let qdrant_conn = Arc::clone(&app_data);
                    let args =
                        BasicAckArguments::new(message.deliver.unwrap().delivery_tag(), false);
                    let _ = channel.basic_ack(args).await;
                    let _ =
                        process_messages(qdrant_conn, message_string, datasource_id.to_string())
                            .await;
                }
            }
        } else {
            println!("There was no stream_id in message... can not upload data!");
        }
    }

    // this is what to do when we get an error
    if let Err(e) = channel.basic_cancel(BasicCancelArguments::new(&ctag)).await {
        println!("error {}", e.to_string());
    };
    Ok(())
    // }
}
