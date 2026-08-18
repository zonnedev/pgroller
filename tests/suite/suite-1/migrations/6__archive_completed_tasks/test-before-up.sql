INSERT INTO accounts (email, full_name) VALUES ('owner6@example.test', 'Owner Six');
INSERT INTO teams (name, owner_account_id) VALUES ('Release', 1);
INSERT INTO projects (team_id, name) VALUES (1, 'Release Train');
INSERT INTO tasks (project_id, title, status) VALUES (1, 'Close sprint', 'done');
