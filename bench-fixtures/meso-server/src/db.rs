use crate::models::{User, Post};

/// In-memory database storing users and posts.
pub struct Database {
    users: Vec<User>,
    posts: Vec<Post>,
    next_user_id: u64,
    next_post_id: u64,
}

impl Database {
    /// Create a new empty database.
    pub fn new() -> Self {
        Database {
            users: Vec::new(),
            posts: Vec::new(),
            next_user_id: 1,
            next_post_id: 1,
        }
    }

    /// Register a new user with the given name.
    pub fn create_user(&self, name: &str) -> User {
        let id = self.next_user_id;
        User {
            id,
            name: name.to_string(),
            email: format!("{}@example.com", name),
        }
    }

    /// Look up a user by their unique identifier.
    pub fn get_user(&self, id: u64) -> Option<&User> {
        self.users.iter().find(|u| u.id == id)
    }

    /// List all posts in the database.
    pub fn list_posts(&self) -> &[Post] {
        &self.posts
    }

    /// Create a new post with the given title.
    pub fn create_post(&self, title: &str) -> Post {
        let id = self.next_post_id;
        Post {
            id,
            title: title.to_string(),
            body: String::new(),
            author_id: 1,
        }
    }
}
