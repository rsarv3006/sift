use crate::handlers::{Handler, home, create_user, get_user, list_posts, create_post};
use crate::db::Database;
use std::collections::HashMap;

/// Router that maps URL path patterns to request handlers.
pub struct RequestRouter {
    routes: HashMap<String, Box<dyn Handler>>,
}

impl RequestRouter {
    /// Create a new router with the default set of routes.
    pub fn new() -> Self {
        let mut routes: HashMap<String, Box<dyn Handler>> = HashMap::new();
        routes.insert("/".to_string(), Box::new(home::HomeHandler));
        routes.insert("/users".to_string(), Box::new(create_user::CreateUserHandler));
        routes.insert("/users/{id}".to_string(), Box::new(get_user::GetUserHandler));
        routes.insert("/posts".to_string(), Box::new(list_posts::ListPostsHandler));
        routes.insert("/posts/create".to_string(), Box::new(create_post::CreatePostHandler));
        RequestRouter { routes }
    }

    /// Find the handler matching the given path, or None if no route matches.
    pub fn route(&self, path: &str) -> Option<&Box<dyn Handler>> {
        if self.routes.contains_key(path) {
            self.routes.get(path)
        } else {
            // Try pattern matching for templated routes
            self.routes.iter()
                .find(|(pattern, _)| path_matches(pattern, path))
                .map(|(_, handler)| handler)
        }
    }
}

/// Check whether a path pattern matches a request path, supporting `{param}` placeholders.
fn path_matches(pattern: &str, path: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();
    if pat_parts.len() != path_parts.len() {
        return false;
    }
    pat_parts.iter().zip(path_parts.iter())
        .all(|(p, s)| p.starts_with('{') || p == s)
}
