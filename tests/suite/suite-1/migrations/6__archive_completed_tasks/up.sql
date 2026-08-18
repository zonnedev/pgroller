UPDATE tasks SET title = '[archived] ' || title WHERE status = 'done';
