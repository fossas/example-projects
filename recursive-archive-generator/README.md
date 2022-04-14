# Recursive archive generator

Generate a directory containing `foo.tar.gz`, which contains another archive called `bar.tar.gz`, which contains some files and another archive called `baz.tar.gz`, which contains some files.

## Usage

```bash
./generate
```

I use it like this when I want to quickly make a directory to scan:

```
./generate; rm -rf ~/fossa/license-scan-dirs/recursive-archive-3; mv recursive-archive ~/fossa/license-scan-dirs/recursive-archive-3
```

The directory structure will be output by the generate command.

`SOMETHING_LICENSE` contains an MIT license. `something.rb` contains an Apache-2.0 license. Everything else is labelled with their license name.

```bash
./generate

WORKING_DIR = /Users/scott/fossa/example-projects/recursive-archive-generator
un-archived directory structure:
recursive-archive
└── vendor
    └── foo
        ├── VERSION
        └── bar
            ├── MIT_LICENSE
            ├── bar_apache.rb
            └── baz
                ├── SOMETHING_LICENSE
                └── something.rb

4 directories, 5 files
after creating foo/bar/baz.tar.gz
recursive-archive
└── vendor
    └── foo
        ├── VERSION
        └── bar
            ├── MIT_LICENSE
            ├── bar_apache.rb
            └── baz.tar.gz

3 directories, 4 files
after creating foo/bar.tar.gz
recursive-archive
└── vendor
    └── foo
        ├── VERSION
        └── bar.tar.gz

2 directories, 2 files

post-archiving directory structure:
recursive-archive
├── fossa-deps.yml
└── vendor
    └── foo.tar.gz

1 directory, 2 files
```
