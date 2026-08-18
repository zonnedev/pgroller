INSERT INTO accounts (email, full_name) VALUES ('owner4@example.test', 'Owner Four');
INSERT INTO teams (name, owner_account_id) VALUES ('Design', 1);
INSERT INTO projects (team_id, name) VALUES (1, 'Design System');
INSERT INTO tasks (project_id, title, status) VALUES (1, 'Review components', 'open');
