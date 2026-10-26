import io, sys, os
path = sys.argv[1]
with io.open(path, 'r', encoding='utf-8') as f:
    content = f.read()
# For now just verify the path works
print(f'File size: {len(content)} chars, path: {path}')
