INSERT INTO accounts (email, full_name) VALUES ('owner2@example.test', 'Owner Two');
INSERT INTO teams (name, owner_account_id) VALUES ('Operations', 1);
INSERT INTO projects (team_id, name) VALUES (1, 'Operations Hub');
