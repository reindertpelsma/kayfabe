function c = kf_err(name, ME)
% A step that threw: a count of 1 (the host expects 0) with the error IDENTIFIER only, never the message (it may carry paths or vendor text). The row goes on with its next step.
c = struct();
c.name = name;
c.kind = 'count';
c.value = 1;
c.expected = 0;
c.guest_pass = false;
c.err_id = ME.identifier;
end
