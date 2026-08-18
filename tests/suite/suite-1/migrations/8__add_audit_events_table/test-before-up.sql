INSERT INTO accounts (email, full_name) VALUES ('owner8@example.test', 'Owner Eight');
INSERT INTO teams (name, owner_account_id) VALUES ('Security', 1);
INSERT INTO projects (team_id, name) VALUES (1, 'Audit Console');
INSERT INTO tasks (project_id, title) VALUES (1, 'Review audit trail');
