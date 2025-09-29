use std::collections::HashMap;
use std::thread;
use trust_tee::prelude::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct User {
    id: u64,
    name: String,
    balance: f64,
    last_login: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Transaction {
    from: u64,
    to: u64,
    amount: f64,
    timestamp: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BankState {
    users: HashMap<u64, User>,
    transactions: Vec<Transaction>,
    next_user_id: u64,
    total_balance: f64,
}

impl BankState {
    fn new() -> Self {
        Self {
            users: HashMap::new(),
            transactions: Vec::new(),
            next_user_id: 1,
            total_balance: 0.0,
        }
    }

    fn create_user(&mut self, name: String, initial_balance: f64) -> u64 {
        let user_id = self.next_user_id;
        self.next_user_id += 1;

        let user = User {
            id: user_id,
            name,
            balance: initial_balance,
            last_login: 0,
        };

        self.total_balance += initial_balance;
        self.users.insert(user_id, user);
        user_id
    }

    fn transfer(&mut self, from: u64, to: u64, amount: f64) -> Result<(), String> {
        // Check if both users exist first
        if !self.users.contains_key(&from) {
            return Err(format!("User {} not found", from));
        }
        if !self.users.contains_key(&to) {
            return Err(format!("User {} not found", to));
        }

        // Check balance
        if let Some(from_user) = self.users.get(&from) {
            if from_user.balance < amount {
                return Err(format!("Insufficient balance: {:.2}", from_user.balance));
            }
        }

        // Perform the transfer
        if let Some(from_user) = self.users.get_mut(&from) {
            from_user.balance -= amount;
        }

        if let Some(to_user) = self.users.get_mut(&to) {
            to_user.balance += amount;
        }

        // Record transaction
        let transaction = Transaction {
            from,
            to,
            amount,
            timestamp: self.transactions.len() as u64,
        };
        self.transactions.push(transaction);
        Ok(())
    }
}

fn main() {
    let bank = Remote::entrust(BankState::new());

    let alice_id = bank.apply_with(
        |state, name: String| state.create_user(name, 1000.0),
        "Alice".to_string(),
    );

    let bob_id = bank.apply_with(
        |state, name: String| state.create_user(name, 500.0),
        "Bob".to_string(),
    );

    let charlie_id = bank.apply_with(
        |state, name: String| state.create_user(name, 200.0),
        "Charlie".to_string(),
    );

    let mut handles = Vec::new();

    for i in 0..5 {
        let bank_clone = bank.clone();
        let handle = thread::spawn(move || {
            let user_id = bank_clone.apply_with(
                |state, (name, balance): (String, f64)| state.create_user(name, balance),
                (format!("ConcurrentUser{}", i), 100.0 * (i + 1) as f64),
            );
            user_id
        });
        handles.push(handle);
    }

    let mut new_user_ids = Vec::new();
    for handle in handles {
        new_user_ids.push(handle.join().unwrap());
    }

    // 5. Stress test with rapid concurrent operations
    let mut stress_handles = Vec::new();

    for _ in 0..10 {
        let bank_worker = bank.clone();
        let handle = thread::spawn(move || {
            let mut success_count = 0;
            let mut fail_count = 0;

            for j in 0..50 {
                // Random transfers between existing users
                let from = if j % 3 == 0 {
                    alice_id
                } else if j % 3 == 1 {
                    bob_id
                } else {
                    charlie_id
                };
                let to = if (j + 1) % 3 == 0 {
                    alice_id
                } else if (j + 1) % 3 == 1 {
                    bob_id
                } else {
                    charlie_id
                };
                let amount = (j % 10 + 1) as f64;

                let result = bank_worker.apply_with(
                    |state, (from, to, amount): (u64, u64, f64)| state.transfer(from, to, amount),
                    (from, to, amount),
                );

                match result {
                    Ok(()) => success_count += 1,
                    Err(_) => fail_count += 1,
                }
            }

            (success_count, fail_count)
        });
        stress_handles.push(handle);
    }

    for handle in stress_handles {
        let _ = handle.join().unwrap();
    }
}
