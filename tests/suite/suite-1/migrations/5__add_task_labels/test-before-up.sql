INSERT INTO accounts (email, full_name) VALUES ('owner5@example.test', 'Owner Five');
INSERT INTO teams (name, owner_account_id) VALUES ('Support', 1);
INSERT INTO projects (team_id, name) VALUES (1, 'Support Desk');
INSERT INTO tasks (project_id, title) VALUES (1, 'Answer ticket');
