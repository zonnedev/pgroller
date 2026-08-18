INSERT INTO audit_events (account_id, event_type, payload) VALUES (1, 'project.created', '{"project":"Audit Console"}');
SELECT pgroller_test.assert_count('audit event stored', 'audit_events', 1);
