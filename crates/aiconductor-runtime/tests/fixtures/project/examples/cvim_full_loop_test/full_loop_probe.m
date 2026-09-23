values = 1:10;
checksum = sum(values);
assert(checksum == 55);
fprintf('FULL_LOOP_MATLAB_OK checksum=%d\n', checksum);
