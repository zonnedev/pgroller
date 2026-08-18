INSERT INTO labels (name) VALUES ('urgent');
INSERT INTO task_labels (task_id, label_id) VALUES (1, 1);
SELECT pgroller_test.assert_count('task label link created', 'task_labels', 1);
