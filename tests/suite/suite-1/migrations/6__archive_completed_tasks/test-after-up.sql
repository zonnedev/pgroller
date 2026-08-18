SELECT pgroller_test.assert_equal('completed task archived', (SELECT title FROM tasks WHERE status = 'done'), '[archived] Close sprint');
