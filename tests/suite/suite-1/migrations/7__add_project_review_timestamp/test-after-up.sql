SELECT pgroller_test.assert_null('review timestamp starts empty', (SELECT last_reviewed_at FROM projects WHERE name = 'Metrics'));
