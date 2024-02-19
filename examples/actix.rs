extern crate slider_captcha_server;
use actix_web::{get, post, web::{self, Data}, App, HttpResponse, HttpServer, Responder};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use serde_json::json;
use slider_captcha_server::{verify_puzzle, SliderPuzzle};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex}};
use mysql::*;
use mysql::prelude::*;
use rand::{thread_rng, Rng};
use std::fs;
use std::env;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let mut app_state = State::default();
    let listeningip = env::var("CAPTCHA_IP").unwrap_or("0.0.0.0".to_string());
    let listeningport = env::var("CAPTCHA_PORT").unwrap_or("18080".to_string());
    let address = format!("{}:{}", listeningip, listeningport);

    // Konstruktion der Datenbank-URL aus Umgebungsvariablen oder direkt
    let database_url = DatabaseConfig::new().url;

    // Testen der Datenbankverbindung
    if let Err(e) = test_database_connection(&database_url) {
        eprintln!("Error when connecting to the database: {}", e);
        std::process::exit(1);
    }

    println!("\nStarted slider_captcha_server on {}.\n", address);
    HttpServer::new(move || {
        App::new()
            .data(app_state.clone())
            .service(generate_handler)
            .service(verify_handler)
    })
    .bind(address)?
    .run()
    .await
}

#[get("/puzzle")]
async fn generate_handler(state: web::Data<State>) -> impl Responder {
    // Path to the directory containing the images
    let dir_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    
    // List all files in the directory and filter out non-files and optionally filter by image extensions
    let images: Vec<PathBuf> = fs::read_dir(dir_path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().map_or(false, |ext| ext == "png" || ext == "jpg"))
        .collect();

    // Select a random image from the list
    let mut rng = thread_rng();
    let random_index = rng.gen_range(0..images.len());
    let image_path = images[random_index].to_str().unwrap();

    let slider_puzzle: SliderPuzzle = match SliderPuzzle::new(image_path) {
        Ok(puzzle) => puzzle,
        Err(err) => {
            eprintln!("!!!BAD IMAGE PATH!!!! \n{}", err);  // Changed to `eprintln!` for error output
            return HttpResponse::InternalServerError().body("Contact Admin.");
        }
    };

    // Generate a unique request ID and store the solution in the global state
    let request_id = uuid::Uuid::new_v4().to_string();
    let solution = slider_puzzle.x;
    state.solutions.lock().unwrap().insert(request_id.clone(), solution);

    let response = json!({
        "puzzle_image": image_to_base64(slider_puzzle.cropped_puzzle),
        "piece_image": image_to_base64(slider_puzzle.puzzle_piece),
        "id": request_id,
        "y": slider_puzzle.y,
    });

    println!("\nSOLUTION:\nid:{:?},\nx:{:?},y:{:?}", request_id, slider_puzzle.x, slider_puzzle.y);
    HttpResponse::Ok().json(response)
}

#[post("/puzzle/solution")]
async fn verify_handler(state: Data<State>, solution: web::Json<Solution>) -> impl Responder {
    // Check if the solution matches the one stored in the global state
    let mut locked_state = state.solutions.lock().unwrap();
    println!("{:?}", locked_state.clone());

    let correct_solution = match locked_state.get(&solution.id) {
        Some(correct_solution) => {
            println!(
                "SOLUTION:\nRequestID:{:?}\nx:{:?}\n",
                solution.id, correct_solution
            );
            *correct_solution
        }
        _ => return HttpResponse::BadRequest().body("Invalid request ID"),
    };
    locked_state.remove(&solution.id);
    if verify_puzzle(correct_solution, solution.x, 0.01) {
                
                let config = DatabaseConfig::new();
                let pool = match Pool::new(config.url) {
                    Ok(pool) => pool,
                    Err(_) => return HttpResponse::InternalServerError().body("Database connection failed"),
                };
                let mut conn = match pool.get_conn() {
                    Ok(conn) => conn,
                    Err(_) => return HttpResponse::InternalServerError().body("Could not establish a connection"),
                };

                let new_uuid = uuid::Uuid::new_v4().to_string();
        
                let query_result = conn.exec_drop(
                    "INSERT INTO approvedid (uuid) VALUES (:uuid)",
                    params! {
                        "uuid" => &new_uuid,
                    },
                );
        
                match query_result {
                    Ok(_) => {
                        let last_insert_id = conn.last_insert_id();
                        let response = serde_json::json!({
                            "message": "VERIFIED!",
                            "uuid": new_uuid,
                            "id": last_insert_id,
                        });
                        HttpResponse::Ok().json(response)
                    },
                    Err(_) => HttpResponse::InternalServerError().body("Database operation failed"),
                }
            
        
    } else {
        HttpResponse::BadRequest().body("Incorrect solution")
    }
}

// A struct to store the global state of the application
#[derive(Clone)]
struct State {
    solutions: Arc<Mutex<HashMap<String, f64>>>,
}

impl Default for State {
    fn default() -> Self {
        State {
            solutions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Solution {
    pub id: String,
    pub x: f64,
}

fn image_to_base64(image: DynamicImage) -> String {
    let mut buffer = Vec::new();
    image
        .write_to(&mut buffer, image::ImageOutputFormat::Png)
        .unwrap();
    base64::encode(&buffer)
}

struct DatabaseConfig {
    url: String,
}

impl DatabaseConfig {
    fn new() -> Self {
        let user = env::var("MYSQL_USER").unwrap_or("myuser".to_string());
        let password = env::var("MYSQL_PASSWORD").unwrap_or("mypassword".to_string());
        let ip = env::var("DB_IP").unwrap_or("172.10.0.3".to_string());
        let port = env::var("DB_PORT").unwrap_or("3306".to_string());
        let database_name = env::var("MYSQL_DATABASE").unwrap_or("mydatabase".to_string());

        DatabaseConfig {
            url: format!("mysql://{}:{}@{}:{}/{}", user, password, ip, port, database_name),
        }
    }
}

fn test_database_connection(database_url: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let pool = mysql::Pool::new(database_url)?;
    let mut conn = pool.get_conn()?;
    conn.query_drop("SELECT 1")?;
    Ok(())
}
