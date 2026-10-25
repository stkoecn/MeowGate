use crate::collector::User;

#[allow(dead_code)]
pub struct RulesGenerator;

#[allow(dead_code)]
impl RulesGenerator {
    /// Generates IN-USER routing rules.
    /// Each rule format: `IN-USER,<username>,node-<node_name>`
    pub fn generate_in_user_rules(users: &[User]) -> Vec<String> {
        let mut rules = Vec::with_capacity(users.len());
        for user in users {
            let group_name = format!("node-{}", user.node_name);
            rules.push(format!("IN-USER,{},{}", user.username, group_name));
        }
        rules
    }
}
