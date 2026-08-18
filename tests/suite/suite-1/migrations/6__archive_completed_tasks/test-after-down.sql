SELECT pgroller_test.assert_equal('irreversible title remains documented', (SELECT title FROM tasks WHERE status = 'done'), '[archived] Close sprint');
