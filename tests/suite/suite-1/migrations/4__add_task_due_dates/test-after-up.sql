SELECT pgroller_test.assert_not_null('due date populated', (SELECT due_date FROM tasks WHERE title = 'Review components'));
