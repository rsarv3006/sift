mod router;
mod handlers;
mod models;
mod db;

use router::RequestRouter;
use handlers::{json_response, not_found};
use models::{User, Post};
use db::Database;

/// The main HTTP server, routing requests through a set of registered handlers.
pub struct Server {
    pub router: RequestRouter,
    pub db: Database,
    pub host: String,
    pub port: u16,
}

impl Server {
    /// Create a new server bound to the given host and port.
    pub fn new(host: &str, port: u16) -> Self {
        Server {
            router: RequestRouter::new(),
            db: Database::new(),
            host: host.to_string(),
            port,
        }
    }

    /// Start listening for incoming connections.
    pub fn start(&self) {
        println!("Server listening on {}:{}", self.host, self.port);
    }

    /// Dispatch a request to the matching handler, or return a 404 response.
    pub fn handle_request(&self, path: &str, body: &str) -> String {
        match self.router.route(path) {
            Some(handler) => handler.handle(path, body, &self.db),
            None => not_found(),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new("127.0.0.1", 8080)
    }
}

fn main() {
    let server = Server::new("0.0.0.0", 3000);
    server.start();
}
