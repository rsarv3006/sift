use crate::db::Database;

/// Handler trait for processing HTTP requests.
pub trait Handler {
    fn handle(&self, path: &str, body: &str, db: &Database) -> String;
}

pub mod home {
    use super::*;
    /// Handler for the home/root endpoint returning a welcome message.
    pub struct HomeHandler;
    impl Handler for HomeHandler {
        fn handle(&self, _path: &str, _body: &str, _db: &Database) -> String {
            json_response("home", "Welcome to the API")
        }
    }
}

pub mod create_user {
    use super::*;
    use crate::models::User;
    /// Handler for registering a new user.
    pub struct CreateUserHandler;
    impl Handler for CreateUserHandler {
        fn handle(&self, _path: &str, body: &str, db: &Database) -> String {
            let user = db.create_user(body);
            json_response("user", &user.name)
        }
    }
}

pub mod get_user {
    use super::*;
    use crate::models::User;
    /// Handler for looking up a user by identifier.
    pub struct GetUserHandler;
    impl Handler for GetUserHandler {
        fn handle(&self, _path: &str, _body: &str, db: &Database) -> String {
            let user = db.get_user(1);
            match user {
                Some(u) => json_response("user", &u.name),
                None => not_found(),
            }
        }
    }
}

pub mod list_posts {
    use super::*;
    use crate::models::Post;
    /// Handler for listing all blog posts.
    pub struct ListPostsHandler;
    impl Handler for ListPostsHandler {
        fn handle(&self, _path: &str, _body: &str, db: &Database) -> String {
            let posts = db.list_posts();
            json_response("posts", &posts.len().to_string())
        }
    }
}

pub mod create_post {
    use super::*;
    use crate::models::Post;
    /// Handler for creating a new blog post.
    pub struct CreatePostHandler;
    impl Handler for CreatePostHandler {
        fn handle(&self, _path: &str, body: &str, db: &Database) -> String {
            let post = db.create_post(body);
            json_response("post", &post.title)
        }
    }
}

/// Format a key-value pair as a simple JSON response string.
pub fn json_response(key: &str, value: &str) -> String {
    format!("{{\"{}\": \"{}\"}}", key, value)
}

/// Return a standard 404 not found response.
pub fn not_found() -> String {
    json_response("error", "not found")
}
