#include <mach-o/dyld.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(void) {
    uint32_t size = 0;
    _NSGetExecutablePath(NULL, &size);
    char *executable = malloc(size);
    if (executable == NULL) {
        perror("Cryptty: allocate launcher path");
        return 1;
    }
    if (_NSGetExecutablePath(executable, &size) != 0) {
        fprintf(stderr, "Cryptty: cannot locate launcher\n");
        free(executable);
        return 1;
    }
    char *resolved = realpath(executable, NULL);
    free(executable);
    if (resolved == NULL) {
        perror("Cryptty: resolve launcher path");
        return 1;
    }
    char *separator = strrchr(resolved, '/');
    *separator = '\0';
    char *script = NULL;
    if (asprintf(&script, "%s/../Resources/launch.sh", resolved) < 0) {
        perror("Cryptty: allocate script path");
        free(resolved);
        return 1;
    }
    free(resolved);
    execl("/bin/sh", "sh", script, (char *)NULL);
    perror("Cryptty: start launcher script");
    free(script);
    return 1;
}
