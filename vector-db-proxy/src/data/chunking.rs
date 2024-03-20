use crate::data::{models::Document, text_splitting::Chunker};
use crate::mongo::models::ChunkingStrategy;
use anyhow::{anyhow, Result};

use lopdf::{Dictionary, Object};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::sync::Arc;

extern crate dotext;

use crate::llm::models::EmbeddingModels;
use dotext::*;
use mongodb::Database;
use qdrant_client::client::QdrantClient;
use tokio::sync::RwLock;
use crate::queue::add_tasks_to_queues::add_message_to_embedding_queue;
use crate::queue::queuing::MyQueue;

pub trait Chunking {
    type Item;
    fn default() -> Self;
    fn dictionary_to_hashmap(&self, dict: &Dictionary) -> HashMap<String, String>;
    fn extract_text_from_pdf(&self, path: String) -> Result<(String, HashMap<String, String>)>;
    fn extract_text_from_docx(&self, path: String) -> Result<(String, HashMap<String, String>)>;
    fn extract_text_from_txt(&self, path: String) -> Result<(String, HashMap<String, String>)>;
    fn detect_pdf_fonts(&self, doc: &lopdf::Document) -> HashMap<String, String>;
    async fn extract_text_from_csv(
        &self,
        path: String,
        datasource_id: String,
        queue: Arc<RwLock<MyQueue<String>>>,
        qdrant_conn: Arc<RwLock<QdrantClient>>,
        mongo_conn: Arc<RwLock<Database>>,
    );
    async fn chunk(
        &self,
        data: String,
        metadata: Option<HashMap<String, String>>,
        strategy: ChunkingStrategy,
        chunking_character: Option<String>,
        embedding_model: EmbeddingModels,
    ) -> Result<Vec<Document>>;
}

pub struct TextChunker;

impl Chunking for TextChunker {
    type Item = u8;

    fn default() -> Self {
        TextChunker
    }

    fn dictionary_to_hashmap(&self, dict: &Dictionary) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (key, value) in dict {
            let key_str = String::from_utf8_lossy(key).into_owned();
            let value_str = match value {
                Object::String(ref s, _) | Object::Name(ref s) => {
                    String::from_utf8_lossy(s).into_owned()
                }
                Object::Integer(i) => i.to_string(),
                Object::Real(f) => f.to_string(),
                Object::Boolean(b) => b.to_string(),
                Object::Array(ref arr) => {
                    // Handling array elements individually
                    let array_str = arr
                        .iter()
                        .map(|obj| match obj {
                            Object::String(ref s, _) | Object::Name(ref s) => {
                                String::from_utf8_lossy(s).into_owned()
                            }
                            Object::Integer(i) => i.to_string(),
                            Object::Real(f) => f.to_string(),
                            Object::Boolean(b) => b.to_string(),
                            _ => "Unknown Type".to_string(),
                        })
                        .collect::<Vec<String>>()
                        .join(", "); // Join elements with a delimiter if needed
                    format!("[{}]", array_str)
                }
                Object::Dictionary(ref dict) => {
                    // Handling nested dictionary
                    let nested_dict = self.dictionary_to_hashmap(dict);
                    serde_json::to_string(&nested_dict)
                        .unwrap_or_else(|_| "Invalid JSON".to_string())
                }
                Object::Stream(ref stream) => {
                    // Handling stream, customize as needed
                    println!("Stream Data: {:?}", stream);
                    "Stream Data".to_string()
                }
                _ => "Unknown Type".to_string(),
            };
            map.insert(key_str, value_str);
        }
        map
    }

    fn extract_text_from_pdf(&self, path: String) -> Result<(String, HashMap<String, String>)> {
        match lopdf::Document::load(path.as_str()) {
            Ok(doc) => {
                let mut metadata = self.detect_pdf_fonts(&doc);
                let mut res = (String::new(), metadata);
                let pages = doc.get_pages();
                if let Some((_, page)) = pages.into_iter().next() {
                    return match pdf_extract::extract_text(path.as_str()) {
                        Ok(text) => {
                            let page_dict = doc.get_dictionary(page)?;
                            metadata = self.dictionary_to_hashmap(page_dict);
                            if text.is_empty() {
                                println!("Text is empty");
                                return Err(anyhow!(
                                    "Unable to extract text from PDF document: {}",
                                    path
                                ));
                            }
                            metadata.insert("character count".to_string(), text.len().to_string());
                            res = (text, metadata);
                            Ok(res)
                        }

                        Err(e) => {
                            println!("An error occurred while extracting text");
                            Err(anyhow!(
                            "An error occurred while trying to extract text from pdf. Error: {}",
                            e
                        ))
                        }
                    };
                }
                println!("Final result from PDF extraction: {:?}", res);
                Ok(res)
            }
            Err(e) => Err(anyhow!(
                "An error occurred while attempting to load PDF doc: {}",
                e
            )),
        }
    }
    // this method covers docx, xlsx and pptx
    fn extract_text_from_docx(&self, path: String) -> Result<(String, HashMap<String, String>)> {
        let metadata = HashMap::new();
        let mut docx = String::new();
        let mut file = Docx::open(path.as_str()).expect("Cannot open file");
        file.read_to_string(&mut docx).unwrap();

        let results = (docx, metadata);
        Ok(results)
    }
    fn extract_text_from_txt(&self, path: String) -> Result<(String, HashMap<String, String>)> {
        let metadata = HashMap::new();
        let mut text = String::new();

        match fs::read_to_string(path) {
            Ok(t) => text = t,
            Err(e) => {
                return Err(anyhow!("Could  not read file. Error: {}", e));
            }
        }

        let results = (text, metadata);
        Ok(results)
    }

    fn detect_pdf_fonts(&self, doc: &lopdf::Document) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        // Iterate over all pages
        for page_id in doc.page_iter() {
            let (resources, _) = doc.get_page_resources(page_id);
            match resources {
                Some(r) => {
                    let fonts = r.get(b"Font").unwrap().as_dict().unwrap();
                    // Iterate over fonts in the resources
                    for (_, font_dict) in fonts {
                        let font_dict = font_dict.as_reference().unwrap();
                        let font_obj = doc.get_object(font_dict).unwrap();

                        if let Object::Dictionary(dict) = font_obj {
                            // Extract font name and encoding
                            let base_font = dict.get(b"BaseFont").unwrap().as_name_str().unwrap();
                            let encoding = dict
                                .get(b"Encoding")
                                .map_or("Unknown", |e| e.as_name_str().unwrap());

                            metadata.insert(base_font.to_string(), encoding.to_string());
                        }
                    }
                }
                None => {
                    println!("Could not retrieve resources from pages! Will be unable to capture font metadata.")
                }
            }
        }
        metadata
    }
    async fn extract_text_from_csv(
        &self,
        path: String,
        datasource_id: String,
        queue: Arc<RwLock<MyQueue<String>>>,
        qdrant_conn: Arc<RwLock<QdrantClient>>,
        mongo_conn: Arc<RwLock<Database>>,
    ) {
        match csv::Reader::from_path(path) {
            Ok(mut rdr) => {
                for row in rdr.records() {
                    match row {
                        Ok(record) => {
                            let string_record = record.iter().collect::<Vec<&str>>().join(", ");
                            let queue = Arc::clone(&queue);
                            let qdrant_conn = Arc::clone(&qdrant_conn);
                            let mongo_conn = Arc::clone(&mongo_conn);
                            let ds_clone = datasource_id.clone();
                            add_message_to_embedding_queue(
                                queue,
                                qdrant_conn,
                                mongo_conn,
                                (ds_clone,
                                 string_record),
                            ).await;
                        }
                        Err(e) => { println!("An error occurred {}", e); }
                    }
                }
            }
            Err(e) => {
                println!("An error occurred: {} ", e);
            }
        }
    }


    async fn chunk(
        &self,
        data: String,
        metadata: Option<HashMap<String, String>>,
        strategy: ChunkingStrategy,
        chunking_character: Option<String>,
        embedding_model: EmbeddingModels,
    ) -> Result<Vec<Document>> {
        let chunker = Chunker::new(embedding_model, true, Some(strategy), chunking_character);
        let doc = Document {
            page_content: data,
            metadata,
            embedding_vector: None,
        };
        let Ok(results) = chunker.split_documents(vec![doc]).await else {
            return Err(anyhow!("Chunker returned an empty document!"));
        };
        Ok(results)
    }
}
