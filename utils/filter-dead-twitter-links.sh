#!/usr/bin/env bash

touch filtered_arts.txt
while IFS= read -r line
do
    if [[ $line == *"twitter.com"* ]]; then
        header=$(curl -o save -w "%{header_json}" "$line" 2> /dev/null)
        echo $header | jq -e '.location[]' >/dev/null
        if [ "$?" = "0" ]; then
            echo "$line" >> filtered_arts.txt
        fi
    else
        echo "$line" >> filtered_arts.txt
    fi
done < $1
