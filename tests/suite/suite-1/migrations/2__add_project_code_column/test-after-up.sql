SELECT pgroller_test.assert_equal('project code generated', (SELECT project_code FROM projects WHERE name = 'Operations Hub'), 'PLATFORM-001');
