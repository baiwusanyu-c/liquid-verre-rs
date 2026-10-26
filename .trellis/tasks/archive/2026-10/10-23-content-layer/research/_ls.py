import os
d = os.path.join(os.environ['TEMP'], 'verre_verify_content')
print(sorted(os.listdir(d)))
