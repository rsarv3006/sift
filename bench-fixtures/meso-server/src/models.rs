/// A user in the system with an identifier, name, and email.
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
}

/// A blog post with title, body content, and author reference.
pub struct Post {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub author_id: u64,
}

/// User role with permission levels for content moderation.
pub enum Role {
    Admin,
    Moderator,
    User,
}

impl Role {
    /// Whether this role can delete content.
    pub fn can_delete(&self) -> bool {
        match self {
            Role::Admin => true,
            Role::Moderator => true,
            Role::User => false,
        }
    }

    /// Whether this role can edit content.
    pub fn can_edit(&self) -> bool {
        matches!(self, Role::Admin | Role::Moderator)
    }
}

/// Trait for converting to JSON representation.
pub trait JsonSerializable {
    fn to_json(&self) -> String;
}

impl JsonSerializable for User {
    fn to_json(&self) -> String {
        format!("{{ \"id\": {}, \"name\": \"{}\" }}", self.id, self.name)
    }
}
