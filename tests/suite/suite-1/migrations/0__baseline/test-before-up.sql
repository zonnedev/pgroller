INSERT INTO accounts (email, full_name) VALUES
    ('owner@example.test', 'Owner Account'),
    ('member@example.test', 'Member Account');

INSERT INTO teams (name, owner_account_id) VALUES ('Platform', 1);
INSERT INTO team_members (team_id, account_id, role) VALUES (1, 1, 'owner'), (1, 2, 'member');
INSERT INTO projects (team_id, name) VALUES (1, 'Migration Platform');
