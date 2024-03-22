use mongodb::Database;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use qdrant_client::client::QdrantClient;
use serde_json::Value;

use crate::llm::models::EmbeddingModels;
use crate::mongo::queries::get_embedding_model_and_embedding_key;
use crate::qdrant::helpers::embed_payload;
use crate::qdrant::utils::Qdrant;
use crate::redis_rs::client::RedisConnection;
use crate::utils::conversions::convert_serde_value_to_hashmap_string;

pub async fn process_messages(
    qdrant_conn: Arc<RwLock<QdrantClient>>,
    mongo_conn: Arc<RwLock<Database>>,
    redis_conn_pool: Arc<Mutex<RedisConnection>>,
    message: String,
    datasource_id: String,
) -> bool {
    // initiate variables
    let mongodb_connection = mongo_conn.read().await;
    let redis_connection = redis_conn_pool.lock().await;
    match serde_json::from_str(message.as_str()) {
        Ok::<Value, _>(message_data) => {
            match get_embedding_model_and_embedding_key(&mongodb_connection, datasource_id.as_str())
                .await
            {
                Ok((model_parameter_result, embedding_field)) => match model_parameter_result {
                    Some(model_parameters) => {
                        let vector_length = model_parameters.embeddingLength as u64;
                        let embedding_model_name = model_parameters.model;
                        let embedding_model_name_clone = embedding_model_name.clone();
                        let ds_clone = datasource_id.clone();
                        let qdrant = Qdrant::new(qdrant_conn, datasource_id);
                        if let Value::Object(data_obj) = message_data {
                            let mut metadata = convert_serde_value_to_hashmap_string(data_obj);
                            if let Some(text_field) = embedding_field {
                                let text = metadata.remove(text_field.as_str()).unwrap();
                                metadata.insert("page_content".to_string(), text.to_owned());
                                match embed_payload(
                                    &metadata,
                                    &text,
                                    Some(ds_clone),
                                    EmbeddingModels::from(embedding_model_name),
                                )
                                    .await
                                {
                                    Ok(point_struct) => {
                                        if let Ok(bulk_upload_result) = qdrant
                                            .bulk_upsert_data(
                                                vec![point_struct],
                                                Some(vector_length),
                                                Some(embedding_model_name_clone),
                                            )
                                            .await
                                        {
                                            let _ = redis_connection.increment_count(&"some_key".to_string(), 1);
                                            return bulk_upload_result;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("An error occurred while upserting  point structs to Qdrant: {}", e);
                                        return false;
                                    }
                                }
                            }
                        } else {
                            return false;
                        }
                    }
                    None => {
                        eprintln!("Model mongo object returned None!");
                        return false;
                    }
                },
                Err(e) => {
                    println!("An error occurred: {}", e);
                    return false;
                }
            }
            false
        }
        Err(e) => {
            eprintln!(
                "An error occurred while attempting to convert message to JSON: {}",
                e
            );
            false
        }
    }
}
